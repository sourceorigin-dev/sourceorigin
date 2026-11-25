//! The lookup through the whole service, as any origin's page or a service
//! calls it.
use crate::{
    routes::directory::ACCOUNTS,
    tests::{Browser, Replicas, json},
};
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
    response::Response,
};
use chrono::Utc;
use serde_json::{Value, json};
use sourceorigin_common::{AccountStatus, UserId, protocol::directory::MAX_LOOKUP_IDS};
use sourceorigin_repository::Database;
use tower::ServiceExt;

/// Asks `replicas` for the accounts `query` names, from another site's page.
async fn lookup(replicas: &Replicas, query: &str) -> Response {
    let request = Request::get(format!("{ACCOUNTS}?{query}"))
        .header(header::ORIGIN, "https://elsewhere.example")
        .body(Body::empty())
        .expect("request");
    replicas.a.clone().oneshot(request).await.expect("response")
}

/// The handle `database` holds for `user` now.
async fn handle(database: &Database, user: UserId) -> String {
    let account = database.users().find_by_id(user).await.expect("lookup");
    account.expect("account").handle.to_string()
}

/// A lookup names each active account by its current handle, in one answer any
/// origin may read, and leaves out an ID no account has. An account that is
/// disabled, scheduled for deletion or purged no longer resolves, and an
/// enabled one does again.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn only_active_accounts_resolve() {
    let replicas = Replicas::new().await;
    let database = Replicas::database().await;
    let (ada, bob) = (
        Browser::new_account(&database).await.user_id,
        Browser::new_account(&database).await.user_id,
    );
    let unknown = UserId::new();
    let query = format!("id={ada}&id={bob}&id={unknown}&id={ada}");
    let found = async || {
        let response = lookup(&replicas, &query).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        let mut accounts = json(response).await["accounts"].take();
        let accounts = accounts.as_array_mut().expect("accounts");
        accounts.sort_by_key(|account| account["id"].to_string());
        std::mem::take(accounts)
    };
    let named = async |user: UserId| json!({ "id": user, "handle": handle(&database, user).await });
    let mut both = vec![named(ada).await, named(bob).await];
    both.sort_by_key(|account| account["id"].to_string());
    assert_eq!(found().await, both);

    let set = async |status: AccountStatus| {
        assert!(
            database
                .users()
                .set_status(bob, status)
                .await
                .expect("status")
        );
    };
    for hidden in [AccountStatus::Disabled, AccountStatus::Deleting] {
        set(hidden).await;
        assert_eq!(found().await, [named(ada).await], "{hidden}");
    }
    set(AccountStatus::Active).await;
    assert_eq!(found().await, both);

    let mut tx = database.begin().await.expect("transaction");
    tx.users()
        .set_status(bob, AccountStatus::Deleting)
        .await
        .expect("status");
    tx.deletions().schedule(bob, Utc::now()).await.expect("due");
    assert!(tx.deletions().purge(bob).await.expect("purge"));
    tx.commit().await.expect("commit");
    assert_eq!(found().await, [named(ada).await]);
}

/// A lookup names at least one account and at most [`MAX_LOOKUP_IDS`], each by
/// an account ID: anything else is refused, at `400`, saying why.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn malformed_lookups_are_refused() {
    let replicas = Replicas::new().await;
    let many = |count: usize| {
        let ids: Vec<_> = (0..count)
            .map(|_| format!("id={}", UserId::new()))
            .collect();
        ids.join("&")
    };
    for (query, why) in [
        (String::new(), "at least one account"),
        ("handle=ada".to_owned(), "at least one account"),
        ("id=ada".to_owned(), "is not an account ID"),
        (many(MAX_LOOKUP_IDS + 1), "at most 100 accounts"),
    ] {
        let response = lookup(&replicas, &query).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
        let body: Value = json(response).await;
        let error = body["error"].as_str().expect("error");
        assert!(error.contains(why), "{error}");
    }
    let response = lookup(&replicas, &many(MAX_LOOKUP_IDS)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json(response).await, json!({ "accounts": [] }));
}
