//! The device authorization grant: a device's codes, its user's decision on the
//! device page, and what each poll answers.

use super::*;
use crate::oidc::device::UserCode;
use sourceorigin_common::clients::GrantTypes;
use sourceorigin_repository::SessionScope;
use std::net::IpAddr;

/// A scenario whose public client is registered for the device grant and for
/// refresh tokens, sending its requests from an address of its own, so its
/// lookups by user code spend no other scenario's budget.
async fn device_flow(trusted: bool) -> Flow {
    let mut flow = Flow::new(false, trusted).await;
    flow.client.grant_types = GrantTypes::new([GrantType::DeviceCode, GrantType::RefreshToken])
        .expect("the device and refresh grants");
    crate::tests::replace(&flow.app.database, &flow.client)
        .await
        .expect("register the device grant");
    flow.context = RequestContext::from_ip(IpAddr::from(*UserId::new().as_uuid().as_bytes()));
    flow
}

impl Flow {
    /// Starts a device authorization for `scope`, and returns its answer.
    async fn device_authorization(&self, scope: &str) -> Value {
        let fields = [("client_id", self.client.id.as_str()), ("scope", scope)];
        let response = self
            .request(
                Method::POST,
                routes::oidc::DEVICE_AUTHORIZATION,
                &fields,
                None,
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        json(response).await
    }

    /// Polls the token endpoint with `device_code`, as the scenario's client.
    async fn poll(&self, device_code: &str) -> Response {
        let fields = [
            ("grant_type", GrantType::DeviceCode.as_str()),
            ("client_id", self.client.id.as_str()),
            ("device_code", device_code),
        ];
        self.request(Method::POST, routes::oidc::TOKEN, &fields, None)
            .await
    }

    /// The error code of a poll with `device_code`.
    async fn poll_error(&self, device_code: &str) -> String {
        let response = self.poll(device_code).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        json(response).await["error"]
            .as_str()
            .expect("an error code")
            .to_owned()
    }

    /// Posts the device page's form: the code typed, and the decision, if any.
    async fn device_page(&self, user_code: &str, decision: Option<&str>) -> Response {
        let fields: Vec<_> = [("user_code", user_code)]
            .into_iter()
            .chain(decision.map(|decision| ("decision", decision)))
            .collect();
        self.request(Method::POST, routes::page::DEVICE, &fields, None)
            .await
    }

    /// The active sessions of the scenario's account that are a device's.
    async fn device_sessions(&self) -> Vec<sourceorigin_repository::StoredSession> {
        (self.app.database.sessions())
            .list_active_for_user(self.user_id)
            .await
            .expect("sessions")
            .into_iter()
            .filter(|session| session.client_id.is_some())
            .collect()
    }
}

/// A user code is eight letters without vowels, read back whatever its case,
/// dashes and spaces, and nothing else is one.
#[test]
fn user_codes_are_read_as_typed() {
    let code = UserCode::parse("bdfg hjkl").expect("a code");
    assert_eq!(code.to_string(), "BDFG-HJKL");
    assert_eq!(UserCode::parse(" BDFG-HJKL\n"), Some(code));
    for typed in ["", "BDFG-HJK", "BDFG-HJKLM", "ADFG-HJKL", "BDFG_HJKL"] {
        assert_eq!(UserCode::parse(typed), None, "{typed}");
    }
}

/// A device gets its codes, waits while its user decides, is slowed down when it
/// polls too soon, and collects its tokens once: the user approves on the device
/// page, which always names the app first, even a trusted one whose link carried
/// the code, since the link only fills the code in. The device signs in to a
/// session of its own. An approval is collected even by a poll that comes before
/// the interval, and a decision is made once.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn a_device_signs_in_once_its_user_approves() {
    let flow = device_flow(true).await;
    let started = flow.device_authorization(REFRESH_SCOPES).await;
    let (device_code, user_code) = (
        started["device_code"].as_str().expect("device code"),
        started["user_code"].as_str().expect("user code"),
    );
    assert_eq!(started["expires_in"], 600);
    assert_eq!(started["interval"], 5);
    assert_eq!(started["verification_uri"], format!("{AUTH_ORIGIN}/device"));
    assert_eq!(
        started["verification_uri_complete"],
        format!("{AUTH_ORIGIN}/device?user_code={user_code}")
    );

    assert_eq!(
        flow.poll_error(device_code).await,
        OAuthCode::AuthorizationPending.as_str()
    );
    assert_eq!(
        flow.poll_error(device_code).await,
        OAuthCode::SlowDown.as_str()
    );

    let link = format!("{}?user_code={user_code}", routes::page::DEVICE);
    let form = html(flow.request(Method::GET, &link, &[], None).await).await;
    assert!(form.contains(&format!(r#"value="{user_code}""#)), "{form}");
    assert!(!form.contains(flow.client.display_name.as_str()), "{form}");

    let approval = html(flow.device_page(&user_code.to_lowercase(), None).await).await;
    assert!(
        approval.contains("Sign in to Test app on your device?"),
        "{approval}"
    );
    assert!(
        approval.contains("Only continue if you started this on your own device."),
        "{approval}"
    );
    assert!(flow.device_sessions().await.is_empty());

    let decided = flow.device_page(user_code, Some("approve")).await;
    assert_eq!(decided.status(), StatusCode::OK);
    let decided = html(decided).await;
    assert!(decided.contains("Your device is signed in"), "{decided}");

    let (tokens, claims) = flow.id_token(flow.poll(device_code).await).await;
    assert!(tokens["refresh_token"].is_string(), "{tokens}");
    let device = flow.device_sessions().await.pop().expect("device session");
    assert_eq!(claims["sid"], device.id.to_string());
    assert_eq!(device.client_id.as_ref(), Some(&flow.client.id));
    assert!(device.expires_at > Utc::now() + chrono::Duration::days(29));
    assert_eq!(
        flow.poll_error(device_code).await,
        OAuthCode::InvalidGrant.as_str()
    );
    let events = flow.events().await;
    assert!(
        events.contains(&DomainEvent::DeviceAuthorized {
            client_id: flow.client.id.clone(),
            session_id: device.id,
        }),
        "{events:?}"
    );
    let again = flow.device_page(user_code, Some("approve")).await;
    assert_eq!(again.status(), StatusCode::NOT_FOUND);
}

/// A device is told when its user refused, and when its code expired before
/// they decided.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn a_refused_or_expired_device_code_says_so() {
    let flow = device_flow(false).await;
    let refused = flow.device_authorization(Scope::OpenId.as_str()).await;
    let user_code = refused["user_code"].as_str().expect("user code");
    let decided = html(flow.device_page(user_code, Some("deny")).await).await;
    assert!(decided.contains("Sign-in cancelled"), "{decided}");
    assert_eq!(
        flow.poll_error(refused["device_code"].as_str().expect("device code"))
            .await,
        OAuthCode::AccessDenied.as_str()
    );
    assert!(flow.device_sessions().await.is_empty());

    let expired = flow.device_authorization(Scope::OpenId.as_str()).await;
    flow.app
        .database
        .oidc()
        .expire_device_authorizations(&flow.client.id)
        .await
        .expect("expire");
    assert_eq!(
        flow.poll_error(expired["device_code"].as_str().expect("device code"))
            .await,
        OAuthCode::ExpiredToken.as_str()
    );
    let page = flow
        .device_page(expired["user_code"].as_str().expect("user code"), None)
        .await;
    assert_eq!(page.status(), StatusCode::NOT_FOUND);
}

/// Only a client registered for the device grant starts a device authorization,
/// and only the client that started one polls it.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn device_codes_belong_to_their_client_and_grant() {
    let flow = device_flow(false).await;
    let other = Flow::new(false, false).await;
    let fields = [
        ("client_id", other.client.id.as_str()),
        ("scope", Scope::OpenId.as_str()),
    ];
    let refused = other
        .request(
            Method::POST,
            routes::oidc::DEVICE_AUTHORIZATION,
            &fields,
            None,
        )
        .await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json(refused).await["error"],
        OAuthCode::UnauthorizedClient.as_str()
    );

    let started = flow.device_authorization(Scope::OpenId.as_str()).await;
    let device_code = started["device_code"].as_str().expect("device code");
    assert_eq!(
        other.poll_error(device_code).await,
        OAuthCode::InvalidGrant.as_str()
    );
    assert_eq!(
        flow.poll_error(device_code).await,
        OAuthCode::AuthorizationPending.as_str()
    );
}

/// A device's session and the browser that approved it end apart: signing out
/// every session but the device's, so the browser's, leaves the device
/// refreshing, and signing the device out leaves the browser signed in.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn a_device_and_its_approving_browser_sign_out_apart() {
    let mut flow = device_flow(false).await;
    let started = flow.device_authorization(REFRESH_SCOPES).await;
    let user_code = started["user_code"].as_str().expect("user code");
    flow.device_page(user_code, Some("approve")).await;
    let tokens = json(
        flow.poll(started["device_code"].as_str().expect("device code"))
            .await,
    )
    .await;
    let device = flow.device_sessions().await.pop().expect("device session");
    let (database, user_id) = (flow.app.database.clone(), flow.user_id);
    let end = async |scope: SessionScope| {
        let mut tx = database.begin().await.expect("transaction");
        let context = RequestContext::TEST.actor(Actor::Anonymous);
        tx.revoke_sessions(&context, user_id, scope, &RevocationReason::UserRevoked)
            .await
            .expect("sessions end");
        tx.commit().await.expect("commit");
    };

    end(SessionScope::AllExcept(device.id)).await;
    let refreshed = flow
        .refresh(tokens["refresh_token"].as_str().expect("refresh token"))
        .await;
    assert_eq!(refreshed.status(), StatusCode::OK);
    let rotated = json(refreshed).await;

    flow.sign_in_again(AuthenticationMethod::Passkey).await;
    end(SessionScope::One(device.id)).await;
    Flow::assert_invalid_grant(
        flow.refresh(rotated["refresh_token"].as_str().expect("rotated token"))
            .await,
    )
    .await;
    let page = flow
        .request(Method::GET, routes::page::DEVICE, &[], None)
        .await;
    assert_eq!(page.status(), StatusCode::OK, "the browser stays signed in");
}

/// Guessing user codes is throttled: a browser's lookups are counted, whatever
/// they find, and refused past ten a minute.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn guessing_user_codes_is_throttled() {
    let flow = device_flow(false).await;
    for _ in 0..10 {
        let guess = flow.device_page("BCDF-GHJK", None).await;
        assert_eq!(guess.status(), StatusCode::NOT_FOUND);
    }
    let throttled = flow.device_page("BCDF-GHJK", None).await;
    assert_eq!(throttled.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        html(throttled).await.contains("Too many codes tried."),
        "the page says why"
    );
}
