//! The client credentials grant (RFC 6749 §4.4): a confidential client's token
//! for itself, for a resource it has access to, and every request it refuses.

use super::*;
use crate::{
    resources::ResourceEdit,
    tests::{api, change_resources, replace},
};
use sourceorigin_common::{
    clients::GrantTypes,
    protocol::claims::{AccessTokenClaims, Audience, AuthenticationClaims, ProfileClaims},
};

impl Flow {
    /// A scenario whose confidential client, untrusted, may use the client
    /// credentials grant beside the code and refresh grants.
    async fn service() -> Self {
        let mut flow = Self::new(true, false).await;
        flow.client.grant_types = GrantTypes::new(GrantType::ALL).expect("every grant");
        replace(&flow.app.database, &flow.client)
            .await
            .expect("client credentials grant");
        flow
    }

    /// Asks for the client's own token with `fields`, authenticating with `basic`.
    async fn own_token(&self, fields: &[(&str, &str)], basic: Option<&str>) -> Response {
        let mut form = vec![("grant_type", GrantType::ClientCredentials.as_str())];
        form.extend_from_slice(fields);
        self.request(Method::POST, "/oauth/token", &form, basic)
            .await
    }

    /// The error the token endpoint answers a client credentials request for
    /// `fields` with, at 400.
    async fn refused(&self, fields: &[(&str, &str)]) -> String {
        let response = self.own_token(fields, self.basic.as_deref()).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{fields:?}");
        let error = json(response).await["error"].as_str().map(str::to_owned);
        error.expect("an OAuth error")
    }

    /// The verified claims of the access token in `tokens`.
    fn own_claims(&self, tokens: &Value) -> AccessTokenClaims {
        let token = tokens["access_token"].as_str().expect("access token");
        let keys = self.app.keys.current().expect("fresh keys");
        keys.verify(token, &self.app.issuer, Utc::now())
            .expect("verify access token")
    }
}

/// A confidential client with the grant gets an access token for the one
/// resource it has access to, with every scope it was granted: its subject is
/// the client, it says nothing of a user, and it comes alone, without a refresh
/// token or an ID token. It may name the resource and narrow the scopes.
/// `UserInfo` refuses the token, which names no user.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn a_client_gets_a_token_for_itself() {
    let flow = Flow::service().await;
    let (_, api) = api(
        &flow.app.database,
        &flow.client.id,
        "notes:read notes:write",
    )
    .await;

    let response = flow.own_token(&[], flow.basic.as_deref()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::PRAGMA], "no-cache");
    let tokens = json(response).await;
    assert_eq!(tokens["scope"], "notes:read notes:write");
    assert_eq!(tokens["expires_in"], ACCESS_TOKEN_SECONDS);
    assert_eq!(tokens["token_type"], AuthScheme::Bearer.as_str());
    for absent in ["refresh_token", "id_token"] {
        assert!(tokens.get(absent).is_none(), "{absent}: {tokens}");
    }
    let claims = flow.own_claims(&tokens);
    assert_eq!(claims.sub, flow.client.id.as_str());
    assert_eq!(claims.client_id, flow.client.id);
    assert_eq!(claims.aud, Audience::One(api.to_string()));
    assert_eq!(claims.authentication, AuthenticationClaims::default());
    assert_eq!(claims.profile, ProfileClaims::default());
    assert_eq!(
        claims.exp - claims.iat,
        chrono::Duration::seconds(ACCESS_TOKEN_SECONDS.into())
    );

    let narrowed = [("resource", api.as_str()), ("scope", "notes:read")];
    let response = flow.own_token(&narrowed, flow.basic.as_deref()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let narrowed = json(response).await;
    assert_eq!(narrowed["scope"], "notes:read");
    assert_eq!(
        flow.own_claims(&narrowed).aud,
        Audience::One(api.to_string())
    );

    let bearer = format!(
        "{} {}",
        AuthScheme::Bearer.as_str(),
        tokens["access_token"].as_str().expect("access token")
    );
    let response = flow
        .request(Method::POST, "/oidc/userinfo", &[], Some(&bearer))
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

/// Each request the grant refuses, with its RFC 6749 error: a client without
/// the grant type, public or confidential, a wrong secret, no resource to
/// target or several, a resource the client has no access to, one of Origin
/// Auth's own scopes, and a scope of the resource it was not granted. Once its
/// access is revoked, the client gets no token at all.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn requests_outside_the_clients_access_are_refused() {
    let unauthorized = OAuthCode::UnauthorizedClient.as_str();
    let public = Flow::new(false, true).await;
    let own = [("client_id", public.client.id.as_str())];
    let response = public.own_token(&own, None).await;
    assert_eq!(json(response).await["error"], unauthorized);
    let unregistered = Flow::new(true, true).await;
    assert_eq!(unregistered.refused(&[]).await, unauthorized);

    let flow = Flow::service().await;
    let database = flow.app.database.clone();
    let target = OAuthCode::InvalidTarget.as_str();
    assert_eq!(flow.refused(&[]).await, target, "no resource at all");
    let (id, api) = api(&database, &flow.client.id, "notes:read").await;
    let wrong = format!("{} bm90ZXM6d3Jvbmc", AuthScheme::Basic.as_str());
    let response = flow.own_token(&[], Some(&wrong)).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        json(response).await["error"],
        OAuthCode::InvalidClient.as_str()
    );

    let (_, theirs) = self::api(&database, &unregistered.client.id, "notes:read").await;
    let scope = OAuthCode::InvalidScope.as_str();
    for (fields, expected) in [
        (&[("resource", theirs.as_str())][..], target),
        (
            &[("resource", "https://unregistered.example/api")][..],
            target,
        ),
        (&[("resource", flow.client.id.as_str())][..], target),
        (&[("scope", "openid")][..], scope),
        (&[("scope", "notes:read notes:write")][..], scope),
        (&[("scope", "")][..], scope),
    ] {
        assert_eq!(flow.refused(fields).await, expected, "{fields:?}");
    }

    let (_, second) = self::api(&database, &flow.client.id, "notes:read").await;
    assert_eq!(flow.refused(&[]).await, target, "two resources, none named");
    let named = [("resource", second.as_str())];
    let response = flow.own_token(&named, flow.basic.as_deref()).await;
    assert_eq!(response.status(), StatusCode::OK);

    let revoke = ResourceEdit::revoke(flow.client.id.as_str()).expect("revoke");
    change_resources(&database, async |resources| {
        resources.edit(id, revoke).await
    })
    .await
    .expect("revoked");
    assert_eq!(flow.refused(&[("resource", api.as_str())]).await, target);
}
