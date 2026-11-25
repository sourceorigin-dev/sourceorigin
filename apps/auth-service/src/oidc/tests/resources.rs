//! Resource indicators (RFC 8707): tokens for an API a client was given access
//! to, the targets and scopes a client may not ask for, and the grants that end
//! with its access.

use super::*;
use crate::{
    resources::ResourceEdit,
    tests::{api, change_resources},
};
use sourceorigin_common::protocol::claims::{AccessTokenClaims, Audience};

/// The identity scopes and a refresh token, with reading the API's notes.
const READ_SCOPES: &str = "openid offline_access notes:read";

impl Flow {
    /// Asks for tokens for `resource` with `scope`.
    async fn authorize_for(&self, resource: &str, scope: &str) -> Response {
        let target = [("resource", resource)];
        self.authorize_with(Method::GET, scope, None, &target).await
    }

    /// The error an authorization request for `resource` with `scope` sends back
    /// to the client.
    async fn refused_for(&self, resource: Option<&str>, scope: &str) -> String {
        let response = match resource {
            Some(resource) => self.authorize_for(resource, scope).await,
            None => self.authorize(scope, None).await,
        };
        parameter(&self.client_redirect(&response), "error")
    }

    /// Asks the token endpoint for `grant`, with the client's ID and `fields`.
    async fn grant(&self, grant: GrantType, fields: &[(&str, &str)]) -> Response {
        let mut form = vec![
            ("grant_type", grant.as_str()),
            ("client_id", self.client.id.as_str()),
        ];
        form.extend_from_slice(fields);
        self.request(Method::POST, "/oauth/token", &form, None)
            .await
    }

    /// The tokens of a code issued for `resource` with `scope`, approving consent
    /// when it is asked.
    async fn tokens_for(&self, resource: &str, scope: &str) -> Value {
        let response = self.authorize_for(resource, scope).await;
        let code = parameter(&self.consent(response).await, "code");
        let fields = [
            ("redirect_uri", self.client.redirect_uris[0].as_str()),
            ("code", code.as_str()),
            ("code_verifier", Self::verifier()),
            ("resource", resource),
        ];
        let response = self.grant(GrantType::AuthorizationCode, &fields).await;
        assert_eq!(response.status(), StatusCode::OK);
        json(response).await
    }

    /// The verified claims of the access token in `tokens`.
    fn access(&self, tokens: &Value) -> AccessTokenClaims {
        let token = tokens["access_token"].as_str().expect("access token");
        let keys = self.app.keys.current().expect("fresh keys");
        keys.verify(token, &self.app.issuer, Utc::now())
            .expect("verify access token")
    }
}

/// A client given access to an API asks for it by its identifier: the consent
/// page names the API and describes its scope, the access token is for the API
/// and the ID token for the client, and a refresh keeps the API as its target,
/// narrows its scopes, and is refused another target without being used up.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn tokens_for_a_resource_carry_its_identifier_and_scopes() {
    let flow = Flow::new(false, false).await;
    let (_, api) = api(&flow.app.database, &flow.client.id, "notes:read").await;
    let (_, other) = self::api(&flow.app.database, &flow.client.id, "notes:read").await;

    let response = flow.authorize_for(api.as_str(), READ_SCOPES).await;
    let consent = location(&response);
    assert_eq!(consent.path(), routes::page::CONSENT);
    let uri = format!("{}?{}", consent.path(), consent.query().expect("query"));
    let page = html(flow.request(Method::GET, &uri, &[], None).await).await;
    assert!(page.contains("will use Notes API as you"), "{page}");
    assert!(page.contains("Read your notes"), "{page}");
    assert!(page.contains(api.as_str()), "{page}");
    assert!(!page.contains(">notes:read<"), "{page}");

    let tokens = flow.tokens_for(api.as_str(), READ_SCOPES).await;
    let access = flow.access(&tokens);
    assert_eq!(access.aud, Audience::One(api.to_string()));
    assert_eq!(access.sub, flow.user_id.to_string());
    assert_eq!(tokens["scope"], "notes:read offline_access openid");
    let keys = flow.app.keys.current().expect("fresh keys");
    let id_token = tokens["id_token"].as_str().expect("ID token");
    let id_token: IdTokenClaims = keys
        .verify(id_token, &flow.app.issuer, Utc::now())
        .expect("verify ID token");
    assert_eq!(id_token.aud, Audience::One(flow.client.id.to_string()));

    let refresh = tokens["refresh_token"].as_str().expect("refresh token");
    let fields = [("refresh_token", refresh), ("resource", other.as_str())];
    let response = flow.grant(GrantType::RefreshToken, &fields).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json(response).await["error"],
        OAuthCode::InvalidTarget.as_str()
    );
    let fields = [
        ("refresh_token", refresh),
        ("resource", api.as_str()),
        ("scope", "openid notes:read"),
    ];
    let response = flow.grant(GrantType::RefreshToken, &fields).await;
    assert_eq!(response.status(), StatusCode::OK);
    let narrowed = json(response).await;
    assert_eq!(narrowed["scope"], "notes:read openid");
    assert_eq!(flow.access(&narrowed).aud, Audience::One(api.to_string()));
}

/// A resource that is not registered, or that the client was not given access
/// to, is `invalid_target`, and so is one that is no identifier. A scope of the
/// resource the client was not granted, or asked without naming the resource, is
/// `invalid_scope`. Each goes back to the client.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn targets_and_scopes_a_client_was_not_granted_are_refused() {
    let flow = Flow::new(false, true).await;
    let stranger = Flow::new(false, true).await;
    let (_, api) = api(&flow.app.database, &flow.client.id, "notes:read").await;
    let (_, theirs) = self::api(&flow.app.database, &stranger.client.id, "notes:read").await;
    let openid = Scope::OpenId.as_str();
    for (resource, scope, expected) in [
        (
            Some("https://unregistered.example/api"),
            openid,
            OAuthCode::InvalidTarget,
        ),
        (Some(theirs.as_str()), openid, OAuthCode::InvalidTarget),
        (Some("notes"), openid, OAuthCode::InvalidTarget),
        (
            Some("https://notes.example/api#x"),
            openid,
            OAuthCode::InvalidTarget,
        ),
        (
            Some(api.as_str()),
            "openid notes:write",
            OAuthCode::InvalidScope,
        ),
        (None, "openid notes:read", OAuthCode::InvalidScope),
    ] {
        assert_eq!(
            flow.refused_for(resource, scope).await,
            expected.as_str(),
            "{resource:?} {scope}"
        );
    }
    assert_eq!(
        flow.access(&flow.tokens_for(api.as_str(), "openid notes:read").await)
            .aud,
        Audience::One(api.to_string())
    );
}

/// A refresh grant holding a scope the client may no longer request is refused,
/// and ending the client's access revokes its grants for the resource, which it
/// can no longer ask for.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn grants_end_with_the_access_they_rely_on() {
    let flow = Flow::new(false, true).await;
    let database = flow.app.database.clone();
    let (id, api) = api(&database, &flow.client.id, "notes:read").await;
    let edit = async |edit: ResourceEdit| {
        change_resources(&database, async |resources| resources.edit(id, edit).await)
            .await
            .expect("edit");
    };
    let refresh = async |tokens: &Value| {
        let token = tokens["refresh_token"].as_str().expect("refresh token");
        let fields = [("refresh_token", token)];
        flow.grant(GrantType::RefreshToken, &fields).await
    };

    let tokens = flow.tokens_for(api.as_str(), READ_SCOPES).await;
    edit(ResourceEdit::grant(flow.client.id.as_str(), "").expect("grant")).await;
    Flow::assert_invalid_grant(refresh(&tokens).await).await;

    let tokens = flow.tokens_for(api.as_str(), REFRESH_SCOPES).await;
    edit(ResourceEdit::revoke(flow.client.id.as_str()).expect("revoke")).await;
    Flow::assert_invalid_grant(refresh(&tokens).await).await;
    assert_eq!(
        flow.refused_for(Some(api.as_str()), Scope::OpenId.as_str())
            .await,
        OAuthCode::InvalidTarget.as_str()
    );
}
