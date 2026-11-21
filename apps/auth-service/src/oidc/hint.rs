//! The `id_token_hint` authorization and logout both read: an ID token auth signed,
//! read the same way by both.

use super::{Claim, OAuthCode, OAuthError, OidcApplication, Reject};
use serde_json::Value;
use sourceorigin_common::{ClientId, SessionId, UserId};
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
    /// Auth can't verify the token: it is malformed, forged, or signed by a key
    /// auth no longer keeps. Logout goes on without it; authorization refuses it.
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
    /// Reads an `id_token_hint`, signed by a published or retired key. Expiry
    /// doesn't count: an expired ID token still names its account, and
    /// RP-Initiated Logout §2 asks logout to accept one. It must still have a
    /// numeric `exp`, and its `sub` must be an account ID.
    pub(super) fn id_token_hint(&self, token: &str) -> Result<IdTokenHint, HintError> {
        let claims = self
            .keys
            .current()
            .map_err(OAuthError::from)?
            .verify_id_token_hint(token, &self.issuer)
            .map_err(HintError::Unverified)?;
        let text = |claim: Claim| claims.get(claim.as_str()).and_then(Value::as_str);
        let client = text(Claim::Audience)
            .unwrap_or_default()
            .parse()
            .map_err(|error| {
                OAuthCode::InvalidRequest
                    .reject_with("id_token_hint audience is not a client ID", error)
            })?;
        if claims
            .get(Claim::ExpiresAt.as_str())
            .and_then(Value::as_i64)
            .is_none()
        {
            return Err(OAuthCode::InvalidRequest
                .reject("id_token_hint has no numeric exp claim")
                .into());
        }
        let user = text(Claim::Subject)
            .ok_or_else(|| OAuthCode::InvalidRequest.reject("id_token_hint has no sub claim"))?
            .parse()
            .map_err(|error| {
                OAuthCode::InvalidRequest
                    .reject_with("id_token_hint sub claim is not an account ID", error)
            })?;
        Ok(IdTokenHint {
            client,
            user,
            session: text(Claim::Session).and_then(|sid| sid.parse().ok()),
        })
    }
}
