//! Google sign-in: `OpenID` Connect, so the identity comes only from a signed ID token.

use super::{
    FederationApplication, FederationConfigError, IdentityStrategy, MAX_TOKEN_BYTES, Provider,
    ProviderEndpoints, ProviderError, VerifiedIdentity,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sourceorigin_auth::{Jws, KeyCache, TokenError};
use sourceorigin_common::protocol::{
    claims::Audience,
    jwt::Algorithm,
    oauth::{Prompt, Scope},
    registered::{ClaimsError, Expiry, RegisteredClaims, Validation},
};
use thiserror::Error;
use url::Url;

/// Google scopes: a signed ID token carrying email and profile claims.
const SCOPES: &[&str] = &[
    Scope::OpenId.as_str(),
    Scope::Email.as_str(),
    Scope::Profile.as_str(),
];
/// Display name proposed when the token carries no usable name.
const FALLBACK_NAME: &str = "Google user";
/// The domain of Gmail addresses, which Google always hosts.
const GMAIL_DOMAIN: &str = "gmail.com";
/// Google's published signing keys.
const KEYS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";

/// Reasons a Google ID token is not accepted.
#[derive(Debug, Error)]
pub(in crate::federation) enum GoogleError {
    /// The ID token is longer than we accept.
    #[error("Google ID token is {0} bytes; at most {MAX_TOKEN_BYTES} are accepted")]
    TokenTooLong(usize),
    /// The ID token is malformed, not RS256, signed by an unpublished key, or its
    /// signature does not verify.
    #[error("Google ID token failed verification")]
    Signature(#[from] TokenError),
    /// The ID token's claims are missing or have the wrong type.
    #[error("Google ID token claims are invalid")]
    Claims(#[source] serde_json::Error),
    /// The ID token is not Google's, not for this client alone, expired, not yet
    /// valid, or for another sign-in attempt.
    #[error("Google ID token claims are not accepted")]
    Rejected(#[from] ClaimsError),
    /// The ID token was authorized for another client.
    #[error("Google ID token authorized party is another client")]
    AuthorizedPartyMismatch,
}

/// Facts needed to request and verify Google ID tokens.
#[derive(Clone)]
pub(in crate::federation) struct GoogleProtocol {
    /// Google's signing keys, cached across sign-ins: kept for the response's
    /// `max-age`, refetched once for an unknown key ID, and fetched at most every 30
    /// seconds. Tests point it at a local fixture.
    pub(in crate::federation) keys: KeyCache,
    /// Accepted `iss` values: Google uses both the HTTPS and the legacy scheme-less form.
    pub(in crate::federation) issuers: &'static [&'static str],
}

impl ProviderEndpoints {
    /// Google's protocol facts, with a cache of its signing keys.
    pub(super) fn google() -> Result<Self, FederationConfigError> {
        Ok(Self {
            issuer: "https://accounts.google.com",
            authorization: "https://accounts.google.com/o/oauth2/v2/auth",
            token: "https://oauth2.googleapis.com/token".into(),
            scopes: SCOPES,
            identity: IdentityStrategy::Google(GoogleProtocol {
                keys: KeyCache::new(
                    Url::parse(KEYS_URL).map_err(FederationConfigError::ProviderEndpoint)?,
                    FederationApplication::http_client(),
                )
                .map_err(FederationConfigError::Keys)?,
                issuers: &["https://accounts.google.com", "accounts.google.com"],
            }),
        })
    }
}

impl GoogleProtocol {
    /// Binds the ID token to this flow's nonce, and makes the user pick an account
    /// instead of silently reusing one.
    pub(super) fn extend_authorization(url: &mut Url, nonce: &str) {
        url.query_pairs_mut()
            .append_pair("nonce", nonce)
            .append_pair("prompt", Prompt::SelectAccount.as_str());
    }
}

impl FederationApplication {
    /// Takes Google's identity only from an `RS256` ID token signed by a published key,
    /// looked up in the protocol's key cache.
    pub(in crate::federation) async fn google_identity(
        &self,
        response: reqwest::Response,
        client_id: &str,
        nonce: &str,
        protocol: &GoogleProtocol,
    ) -> Result<VerifiedIdentity, ProviderError> {
        let token: GoogleToken = Self::json(response).await?;
        let claims =
            GoogleClaims::verify(&token.id_token, protocol, client_id, nonce, Utc::now()).await?;
        let authoritative = claims.authoritative();
        VerifiedIdentity::new(
            Provider::Google,
            claims.sub,
            (
                claims.email.filter(|_| claims.email_verified),
                authoritative,
            ),
            (claims.name, FALLBACK_NAME.into()),
            claims.picture.as_deref(),
        )
    }
}

/// Google's code-exchange response.
#[derive(Deserialize)]
struct GoogleToken {
    /// Signed ID token, never trusted before `GoogleClaims::verify`.
    id_token: String,
}

/// Claims read from a Google ID token.
#[derive(Deserialize)]
pub(in crate::federation) struct GoogleClaims {
    /// Who issued the token; must be one of Google's issuers.
    iss: String,
    /// Intended client or clients.
    aud: Audience,
    /// Stable Google account ID.
    sub: String,
    /// Expiry.
    #[serde(with = "chrono::serde::ts_seconds")]
    exp: DateTime<Utc>,
    /// Time before which the token must not be accepted, which Google may send.
    #[serde(default, with = "chrono::serde::ts_seconds_option")]
    nbf: Option<DateTime<Utc>>,
    /// Issuance time.
    #[serde(with = "chrono::serde::ts_seconds")]
    iat: DateTime<Utc>,
    /// Nonce stored with the federation flow when it started.
    nonce: String,
    /// Authorized party; must be this client when present.
    azp: Option<String>,
    /// Contact address, if the `email` scope returned one.
    email: Option<String>,
    /// Whether Google verified `email`.
    #[serde(default)]
    email_verified: bool,
    /// The Google Workspace domain of the account, if it is one.
    hd: Option<String>,
    /// Profile name proposed for onboarding.
    name: Option<String>,
    /// The profile picture's URL, with the `profile` scope.
    picture: Option<String>,
}

impl GoogleClaims {
    /// Verifies the signature against Google's published keys, then, at `now`, the
    /// shared check of the issuer, exclusive audience, times and nonce, and last the
    /// authorized party. Unlike the SDK's own ID tokens, expiry has no leeway
    /// ([`Expiry::Strict`]): the token came straight from Google's token endpoint.
    pub(in crate::federation) async fn verify(
        token: &str,
        protocol: &GoogleProtocol,
        client_id: &str,
        nonce: &str,
        now: DateTime<Utc>,
    ) -> Result<Self, GoogleError> {
        if token.len() > MAX_TOKEN_BYTES {
            return Err(GoogleError::TokenTooLong(token.len()));
        }
        let jws = Jws::parse(token)?;
        let algorithm = jws.require_algorithm(&[Algorithm::Rs256])?;
        protocol.keys.verify(&jws, algorithm).await?;
        let claims: Self = serde_json::from_slice(jws.payload()).map_err(GoogleError::Claims)?;
        Validation {
            issuers: protocol.issuers,
            audience: Some(client_id),
            nonce: Some(nonce),
            expiry: Expiry::Strict,
            now,
        }
        .check(&claims)?;
        if claims.azp.as_deref().is_some_and(|azp| azp != client_id) {
            return Err(GoogleError::AuthorizedPartyMismatch);
        }
        Ok(claims)
    }

    /// Whether Google is authoritative for `email`, as Google documents: a Gmail
    /// address, or one under the account's Workspace domain. Google verified other
    /// addresses once, and they may have changed owners since.
    pub(in crate::federation) fn authoritative(&self) -> bool {
        let Some((_, domain)) = self
            .email
            .as_deref()
            .and_then(|email| email.rsplit_once('@'))
        else {
            return false;
        };
        self.email_verified
            && (domain.eq_ignore_ascii_case(GMAIL_DOMAIN)
                || self
                    .hd
                    .as_deref()
                    .is_some_and(|hd| domain.eq_ignore_ascii_case(hd)))
    }
}

impl RegisteredClaims for GoogleClaims {
    /// An ID token, which trusts no other audience (OIDC Core §3.1.3.7).
    const EXCLUSIVE_AUDIENCE: bool = true;

    fn issuer(&self) -> &str {
        &self.iss
    }
    fn audience(&self) -> &Audience {
        &self.aud
    }
    fn expires_at(&self) -> DateTime<Utc> {
        self.exp
    }
    fn not_before(&self) -> Option<DateTime<Utc>> {
        self.nbf
    }
    fn issued_at(&self) -> DateTime<Utc> {
        self.iat
    }
    fn nonce(&self) -> Option<&str> {
        Some(&self.nonce)
    }
}
