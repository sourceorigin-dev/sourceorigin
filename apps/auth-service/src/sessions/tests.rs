//! Database-backed session scenarios: whose sessions the list shows, and which
//! ones each sign-out ends.

use super::*;
use crate::{
    account::AccountApplication,
    routes,
    tests::{Browser, Replicas, refresh_grant, register, registration},
};
use axum::{
    Router,
    http::{Method, StatusCode},
};
use chrono::Duration;
use serde_json::{Value, json};
use sourceorigin_common::{
    TokenId,
    clients::ClientKind,
    events::{Actor, DomainEvent, RevocationReason},
};
use sourceorigin_repository::{Database, InsertSession};

/// The session routes over the test database.
struct Harness {
    /// Routes under test.
    router: Router,
    /// The scenario database.
    database: Database,
}

impl Harness {
    /// Builds the session routes over a migrated test database.
    async fn new() -> Self {
        let database = Replicas::database().await;
        Self {
            router: AccountApplication::new(database.clone()).routes(),
            database,
        }
    }

    /// The `(id, current)` pairs of `browser`'s session list, in its order.
    async fn list(&self, browser: &Browser) -> Vec<(Value, bool)> {
        let (status, list) = browser
            .call(&self.router, Method::GET, routes::sessions::LIST)
            .await;
        assert_eq!(status, StatusCode::OK, "{list}");
        list.as_array()
            .expect("a list")
            .iter()
            .map(|item| (item["id"].clone(), item["current"] == true))
            .collect()
    }

    /// Signs `session` out as `browser`.
    async fn revoke(&self, browser: &Browser, session: SessionId) -> StatusCode {
        let path = routes::sessions::ITEM.replace("{id}", &session.to_string());
        browser.call(&self.router, Method::DELETE, &path).await.0
    }

    /// Whether `session` is still active.
    async fn active(&self, session: SessionId) -> bool {
        self.database
            .sessions()
            .find_active_by_id(session)
            .await
            .expect("lookup")
            .is_some()
    }
}

/// The list shows the caller's active sessions, most recently active first, with
/// the device, IP and methods of each, and marks the caller's own.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn the_list_shows_only_the_callers_sessions() {
    let harness = Harness::new().await;
    let laptop = Browser::new_account(&harness.database).await;
    let phone = Browser::sign_in(&harness.database, laptop.user_id).await;
    let stranger = Browser::new_account(&harness.database).await;

    assert_eq!(
        harness.list(&laptop).await,
        [
            (json!(phone.session_id), false),
            (json!(laptop.session_id), true)
        ]
    );
    let (_, list) = laptop
        .call(&harness.router, Method::GET, routes::sessions::LIST)
        .await;
    assert_eq!(
        list[0]["device"],
        json!({"browser": "firefox", "platform": "linux"})
    );
    assert_eq!(list[0]["ip"], "192.0.2.1");
    assert_eq!(list[0]["methods"], json!(["passkey"]));
    assert_eq!(
        harness.list(&stranger).await,
        [(json!(stranger.session_id), true)]
    );
}

/// One session is signed out with its own sign-out event, attributed to the
/// caller. The caller's own session and another account's are refused.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn signing_out_one_session_ends_only_that_one() {
    let harness = Harness::new().await;
    let laptop = Browser::new_account(&harness.database).await;
    let phone = Browser::sign_in(&harness.database, laptop.user_id).await;
    let stranger = Browser::new_account(&harness.database).await;

    assert_eq!(
        harness.revoke(&laptop, laptop.session_id).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        harness.revoke(&laptop, stranger.session_id).await,
        StatusCode::NOT_FOUND
    );
    assert!(harness.active(stranger.session_id).await);

    assert_eq!(
        harness.revoke(&laptop, phone.session_id).await,
        StatusCode::NO_CONTENT
    );
    assert!(!harness.active(phone.session_id).await);
    assert!(harness.active(laptop.session_id).await);
    assert_eq!(
        harness.revoke(&laptop, phone.session_id).await,
        StatusCode::NOT_FOUND,
        "a signed-out session is gone"
    );
    let latest = harness
        .database
        .events()
        .history_for_user(laptop.user_id, 1)
        .await
        .expect("history")
        .remove(0);
    assert_eq!(
        latest.actor.known(),
        Some(&Actor::User {
            user_id: laptop.user_id,
            session_id: Some(laptop.session_id),
        })
    );
    assert_eq!(
        latest.event.known(),
        Some(&DomainEvent::SessionRevoked {
            session_id: phone.session_id,
            reason: RevocationReason::UserRevoked,
        })
    );
}

/// Signing out everywhere else ends every other session of the account, says how
/// many, and keeps the caller's and other accounts' sessions.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn signing_out_others_keeps_the_current_session() {
    let harness = Harness::new().await;
    let laptop = Browser::new_account(&harness.database).await;
    let phone = Browser::sign_in(&harness.database, laptop.user_id).await;
    let tablet = Browser::sign_in(&harness.database, laptop.user_id).await;
    let stranger = Browser::new_account(&harness.database).await;

    let (status, body) = laptop
        .call(
            &harness.router,
            Method::POST,
            routes::sessions::REVOKE_OTHERS,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({"revoked": 2}));
    for session in [phone.session_id, tablet.session_id] {
        assert!(!harness.active(session).await);
    }
    assert_eq!(
        harness.list(&laptop).await,
        [(json!(laptop.session_id), true)]
    );
    assert!(harness.active(stranger.session_id).await);
}

/// Signing out everywhere else also ends the refresh grants of the account's
/// sessions that already expired, which outlive them, and keeps the grants of the
/// caller's own session.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn signing_out_others_ends_the_grants_of_expired_sessions() {
    let harness = Harness::new().await;
    let laptop = Browser::new_account(&harness.database).await;
    let hour_ago = Utc::now() - Duration::hours(1);
    let expired = harness
        .database
        .sessions()
        .insert(&InsertSession {
            id: SessionId::new(),
            user_id: laptop.user_id,
            authentication_methods: vec![AuthenticationMethod::Passkey],
            authenticated_at: hour_ago - Duration::hours(1),
            expires_at: hour_ago,
            user_agent_family: RequestContext::TEST.user_agent_family,
            ip: RequestContext::TEST.client_ip,
            passkey_confirmed_at: None,
            passkey_id: None,
        })
        .await
        .expect("expired session");
    let client = registration(ClientKind::Public, "https://client.example");
    register(&harness.database, &client).await.expect("client");
    let (kept, ended) = (TokenId::new(), TokenId::new());
    for (session, family) in [(laptop.session_id, kept), (expired.id, ended)] {
        refresh_grant(
            &harness.database,
            laptop.user_id,
            session,
            &client.id,
            family,
        )
        .await;
    }

    let (status, body) = laptop
        .call(
            &harness.router,
            Method::POST,
            routes::sessions::REVOKE_OTHERS,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let grants = harness
        .database
        .tokens()
        .list_for_user(laptop.user_id)
        .await
        .expect("grants");
    let revoked = |family| {
        grants
            .iter()
            .find(|grant| grant.id == family)
            .expect("grant")
            .revoked_at
            .is_some()
    };
    assert!(revoked(ended), "the expired session's grant ends");
    assert!(!revoked(kept), "the caller's own grant keeps working");
}
