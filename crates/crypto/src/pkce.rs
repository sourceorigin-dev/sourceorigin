//! PKCE S256 (RFC 7636): the challenge every client sends, and, for auth, the
//! check of the verifier against it.

use super::Digest;
#[cfg(feature = "server")]
use super::{CryptoError, Secret};

/// RFC 7636 minimum verifier length.
#[cfg(feature = "server")]
const MIN_VERIFIER_LENGTH: usize = 43;
/// RFC 7636 maximum verifier length.
#[cfg(feature = "server")]
const MAX_VERIFIER_LENGTH: usize = 128;

/// PKCE S256 operations, shared by the OIDC provider, its social-login client and
/// the SDK.
pub struct Pkce;

impl Pkce {
    /// The S256 challenge of `verifier`: the unpadded base64url of its SHA-256. A
    /// client sends it with a verifier it generated, such as a [`Secret`]'s
    /// encoding, which is always valid.
    ///
    /// ```
    /// use sourceorigin_crypto::Pkce;
    ///
    /// // RFC 7636 appendix B.
    /// assert_eq!(
    ///     Pkce::challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
    ///     "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
    /// );
    /// ```
    ///
    /// [`Secret`]: crate::Secret
    #[must_use]
    pub fn challenge(verifier: &str) -> String {
        Digest::sha256(verifier.as_bytes()).encoded()
    }

    /// Checks that a verifier a client presents follows RFC 7636 and matches an S256
    /// challenge.
    ///
    /// # Errors
    /// [`CryptoError::PkceVerifier`] for a verifier of invalid length or characters;
    /// [`CryptoError::PkceMismatch`] when it does not match.
    #[cfg(feature = "server")]
    pub fn verify(verifier: &str, challenge: &str) -> Result<(), CryptoError> {
        if !(MIN_VERIFIER_LENGTH..=MAX_VERIFIER_LENGTH).contains(&verifier.len())
            || !verifier.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
            })
        {
            return Err(CryptoError::PkceVerifier);
        }
        if Digest::constant_time_eq(Self::challenge(verifier).as_bytes(), challenge.as_bytes()) {
            Ok(())
        } else {
            Err(CryptoError::PkceMismatch)
        }
    }

    /// Checks that an incoming challenge encodes a SHA-256 result.
    #[cfg(feature = "server")]
    #[must_use]
    pub fn valid_challenge(challenge: &str) -> bool {
        Secret::parse(challenge).is_ok()
    }
}
