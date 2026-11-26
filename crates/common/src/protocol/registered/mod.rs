//! The one check of a token's registered claims (RFC 7519 §4.1) and `nonce`, which
//! auth runs on its own tokens and Google's, and the SDKs on auth's.
//!
//! The leeway policy: `nbf` and `iat` always allow [`LEEWAY`], since the issuer's
//! clock may run ahead of the verifier's. `exp` allows it only where [`Expiry`]
//! says, since lenience there only extends a token's life.
//!
//! [`LEEWAY`]: crate::protocol::constants::LEEWAY
//! [`Expiry`]: crate::protocol::registered::Expiry
//!
//! ```
//! use chrono::{TimeDelta, Utc};
//! use sourceorigin_common::protocol::{
//!     claims::IdTokenClaims,
//!     registered::{ClaimsError, Expiry, Validation},
//! };
//!
//! let claims: IdTokenClaims = serde_json::from_str(r#"{
//!     "iss": "https://auth.sourceorigin.dev",
//!     "sub": "01936f3e-8c00-7000-8000-000000000001",
//!     "aud": "notes-web",
//!     "iat": 1767225540,
//!     "exp": 1767225840,
//!     "nonce": "n-0S6_WzA2Mj"
//! }"#)?;
//! let validation = Validation {
//!     issuers: &["https://auth.sourceorigin.dev"],
//!     audience: Some("notes-web"),
//!     nonce: Some("n-0S6_WzA2Mj"),
//!     expiry: Expiry::Lenient,
//!     now: claims.iat + TimeDelta::minutes(1),
//! };
//! assert_eq!(validation.check(&claims), Ok(()));
//!
//! // Ten seconds past `exp`, within the leeway an SDK allows, but not auth.
//! let late = Validation { now: claims.exp + TimeDelta::seconds(10), ..validation };
//! assert_eq!(late.check(&claims), Ok(()));
//! let strict = Validation { expiry: Expiry::Strict, ..late };
//! assert_eq!(strict.check(&claims), Err(ClaimsError::Expired(claims.exp)));
//! # Ok::<(), serde_json::Error>(())
//! ```

use super::{
    claims::{AccessTokenClaims, Audience, IdTokenClaims, LogoutTokenClaims},
    constants::LEEWAY,
    jwt::TokenType,
    sdk::ErrorReason,
};
use chrono::{DateTime, TimeDelta, Utc};
use thiserror::Error;

/// How [`Validation::check`] reads `exp`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expiry {
    /// Expired [`LEEWAY`] after `exp`: the SDKs, whose clock may run ahead of
    /// auth's.
    Lenient,
    /// Expired at `exp`: auth, which ends its own tokens when it said they end, and
    /// Google's ID tokens, which come straight from Google's token endpoint.
    Strict,
    /// Never expired: an `id_token_hint`, which still names its account once expired
    /// (RP-Initiated Logout 1.0 §2).
    Ignored,
}

impl Expiry {
    /// How long after `exp` a token still verifies, or `None` if it always does.
    const fn leeway(self) -> Option<TimeDelta> {
        match self {
            Self::Lenient => Some(LEEWAY),
            Self::Strict => Some(TimeDelta::zero()),
            Self::Ignored => None,
        }
    }
}

/// What a token's registered claims must say, checked at `now`.
#[derive(Clone, Copy, Debug)]
pub struct Validation<'a> {
    /// The accepted `iss` values: one, except Google, which spells its issuer two ways.
    pub issuers: &'a [&'a str],
    /// The recipient `aud` must name, or `None` for any: auth's `UserInfo` endpoint
    /// accepts every access token auth issued, whoever it is for.
    pub audience: Option<&'a str>,
    /// The `nonce` the token must carry, when the sign-in sent one.
    pub nonce: Option<&'a str>,
    /// How `exp` is read.
    pub expiry: Expiry,
    /// The time to check at, from the caller's clock, so tests can pin it.
    pub now: DateTime<Utc>,
}

impl Validation<'_> {
    /// Checks `iss`, `aud`, `exp`, `nbf`, `iat` and `nonce`, in that order, so a
    /// token with several faults gets the first one's error.
    ///
    /// # Errors
    /// The first claim that fails, as [`ClaimsError`].
    pub fn check<T: RegisteredClaims>(&self, claims: &T) -> Result<(), ClaimsError> {
        if !self.issuers.contains(&claims.issuer()) {
            return Err(ClaimsError::WrongIssuer {
                expected: self.issuers.iter().map(ToString::to_string).collect(),
                found: claims.issuer().to_owned(),
            });
        }
        if let Some(audience) = self.audience {
            let found = claims.audience();
            let others = T::EXCLUSIVE_AUDIENCE && found.iter().any(|value| value != audience);
            if others || !found.contains(audience) {
                return Err(ClaimsError::WrongAudience {
                    expected: audience.to_owned(),
                    found: found.clone(),
                });
            }
        }
        let expired = self.expiry.leeway().is_some_and(|leeway| {
            claims
                .expires_at()
                .checked_add_signed(leeway)
                .is_some_and(|deadline| self.now >= deadline)
        });
        if expired {
            return Err(ClaimsError::Expired(claims.expires_at()));
        }
        let future = self.now + LEEWAY;
        if let Some(not_before) = claims.not_before().filter(|nbf| *nbf > future) {
            return Err(ClaimsError::NotYetValid(not_before));
        }
        if claims.issued_at() > future {
            return Err(ClaimsError::IssuedInFuture(claims.issued_at()));
        }
        if self
            .nonce
            .is_some_and(|nonce| claims.nonce() != Some(nonce))
        {
            return Err(ClaimsError::NonceMismatch);
        }
        Ok(())
    }
}

/// Why [`Validation::check`] refused a token's claims.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClaimsError {
    /// `iss` is none of the accepted issuers.
    #[error("token issuer is {found:?}, expected one of {expected:?}")]
    WrongIssuer {
        /// The accepted issuers.
        expected: Vec<String>,
        /// The token's `iss`.
        found: String,
    },
    /// `aud` does not name the expected recipient, or names others where it must
    /// name that one alone.
    #[error("token audience {found:?} does not contain {expected:?}")]
    WrongAudience {
        /// The resource or client this token must be for.
        expected: String,
        /// The token's `aud`.
        found: Audience,
    },
    /// `exp` has passed, beyond any leeway.
    #[error("token expired at {0}")]
    Expired(DateTime<Utc>),
    /// `nbf` is in the future, beyond the leeway.
    #[error("token is not valid before {0}")]
    NotYetValid(DateTime<Utc>),
    /// `iat` is in the future, beyond the leeway.
    #[error("token is issued in the future, at {0}")]
    IssuedInFuture(DateTime<Utc>),
    /// The ID token's `nonce` is not the sign-in's.
    #[error("ID token nonce does not match this login")]
    NonceMismatch,
}

impl ClaimsError {
    /// The shared SDK error reason.
    #[must_use]
    pub const fn reason(&self) -> ErrorReason {
        match self {
            Self::WrongIssuer { .. } => ErrorReason::IssuerMismatch,
            Self::WrongAudience { .. } => ErrorReason::WrongAudience,
            Self::Expired(_) => ErrorReason::Expired,
            Self::NotYetValid(_) => ErrorReason::NotYetValid,
            Self::IssuedInFuture(_) => ErrorReason::IssuedInFuture,
            Self::NonceMismatch => ErrorReason::NonceMismatch,
        }
    }
}

/// A token's type, registered claims and `nonce`, as [`Validation::check`] reads
/// them.
pub trait RegisteredClaims {
    /// The `typ` header of the tokens that carry these claims, so a verifier reads
    /// claims only from a token of their type.
    const TYPE: TokenType;

    /// Whether `aud` must name the expected recipient alone. ID tokens trust no
    /// other audience (OIDC Core §3.1.3.7), so `azp` can only confirm the client,
    /// and auth addresses each logout token to one client; access tokens may name
    /// several.
    const EXCLUSIVE_AUDIENCE: bool;

    /// `iss`.
    fn issuer(&self) -> &str;
    /// `aud`.
    fn audience(&self) -> &Audience;
    /// `exp`.
    fn expires_at(&self) -> DateTime<Utc>;
    /// `nbf`, if any.
    fn not_before(&self) -> Option<DateTime<Utc>>;
    /// `iat`.
    fn issued_at(&self) -> DateTime<Utc>;
    /// `nonce`, if any.
    fn nonce(&self) -> Option<&str>;
}

/// Implements [`RegisteredClaims`] for claims types whose registered claims are their
/// `iss`, `aud`, `exp` and `iat` fields. Each names its token type, and its `nbf`
/// and `nonce` fields, or
/// `none` for a type without one, so no type can leave a check out by omission.
macro_rules! registered_claims {
    (@nbf $claims:ident, none) => {
        None
    };
    (@nbf $claims:ident, $field:ident) => {
        $claims.$field
    };
    (@nonce $claims:ident, none) => {
        None
    };
    (@nonce $claims:ident, $field:ident) => {
        $claims.$field.as_deref()
    };
    ($(
        impl $([$($generics:tt)*])? for $claims:ty {
            type: $type:ident, exclusive_audience: $exclusive:literal, not_before: $nbf:ident,
            nonce: $nonce:ident
        }
    )+) => {$(
        impl$(<$($generics)*>)? RegisteredClaims for $claims {
            const TYPE: TokenType = TokenType::$type;
            const EXCLUSIVE_AUDIENCE: bool = $exclusive;

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
                registered_claims!(@nbf self, $nbf)
            }
            fn issued_at(&self) -> DateTime<Utc> {
                self.iat
            }
            fn nonce(&self) -> Option<&str> {
                registered_claims!(@nonce self, $nonce)
            }
        }
    )+};
}

registered_claims! {
    impl [X] for AccessTokenClaims<X> {
        type: Access, exclusive_audience: false, not_before: nbf, nonce: none
    }
    impl for IdTokenClaims {
        type: Id, exclusive_audience: true, not_before: nbf, nonce: nonce
    }
    impl for LogoutTokenClaims {
        type: Logout, exclusive_audience: true, not_before: none, nonce: none
    }
}

#[cfg(test)]
mod tests;
