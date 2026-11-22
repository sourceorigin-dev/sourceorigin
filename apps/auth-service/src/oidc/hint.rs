//! The `id_token_hint` authorization and logout both read: an ID token auth signed,
//! read the same way by both.

use super::{OAuthCode, OAuthError, OidcApplication, Reject};
use chrono::Utc;
use sourceorigin_common::{ClientId, SessionId, UserId, protocol::claims::Audience};
use sourceorigin_crypto::CryptoError;

/// What a verified `id_token_hint` says: the client auth issued it to, and the
/// account and session it names.
pub(super) struct IdTokenHint {
    /// The token's audience.
    pub(super) client: ClientId,
    /// The token's `sub`.
    pub(super) user: UserId,
    /// The token's `sid`, if it has one auth can read.
    pub(super) session: Option<SessionId>,
}

/// Why an `id_token_hint` was not read.
pub(super) enum HintError {
    /// Auth can't verify the token: it is malformed, forged, signed by a key auth
    /// no longer keeps, or names another issuer or a future `nbf` or `iat`. Logout
    /// goes on without it; authorization refuses it.
    Unverified(CryptoError),
    /// The request is refused: auth's keys are unavailable, or auth signed the
    /// token but its claims are not ones auth issues.
    Refused(OAuthError),
}

impl From<OAuthError> for HintError {
    fn from(error: OAuthError) -> Self {
        Self::Refused(error)
    }
}

impl From<HintError> for OAuthError {
    /// Authorization refuses any hint it can't read.
    fn from(error: HintError) -> Self {
        match error {
            HintError::Unverified(error) => OAuthCode::InvalidRequest
                .reject_with(format!("id_token_hint is invalid: {error}"), error),
            HintError::Refused(error) => error,
        }
    }
}

impl OidcApplication {
    /// Reads an `id_token_hint`, signed by a published or retired key, whose issuer
    /// and times pass the shared check. Expiry doesn't count: an expired ID token
    /// still names its account, and RP-Initiated Logout §2 asks logout to accept
    /// one. Its claims must still decode as an ID token's, a numeric `exp`
    /// included, name one client, and its `sub` must be an account ID.
    pub(super) fn id_token_hint(&self, token: &str) -> Result<IdTokenHint, HintError> {
        let claims = self
            .keys
            .current()
            .map_err(OAuthError::from)?
            .verify_id_token_hint(token, &self.issuer, Utc::now())
            .map_err(|error| match error {
                CryptoError::InvalidClaims(error) => HintError::Refused(
                    OAuthCode::InvalidRequest
                        .reject_with(format!("id_token_hint claims are invalid: {error}"), error),
                ),
                error => HintError::Unverified(error),
            })?;
        let Audience::One(client) = &claims.aud else {
            return Err(OAuthCode::InvalidRequest
                .reject("id_token_hint audience is not a client ID")
                .into());
        };
        let client = client.parse().map_err(|error| {
            OAuthCode::InvalidRequest
                .reject_with("id_token_hint audience is not a client ID", error)
        })?;
        let user = claims.sub.parse().map_err(|error| {
            OAuthCode::InvalidRequest
                .reject_with("id_token_hint sub claim is not an account ID", error)
        })?;
        Ok(IdTokenHint {
            client,
            user,
            session: claims.authentication.sid,
        })
    }
}
