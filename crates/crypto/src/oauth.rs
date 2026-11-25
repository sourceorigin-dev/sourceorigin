//! What auth checks of OAuth clients' secrets.

use super::{CryptoError, Digest, Secret};
use secrecy::{ExposeSecret, SecretString, zeroize::Zeroizing};
use std::fmt;

/// Starts every generated client secret, so secret scanners can recognize leaked ones.
pub(crate) const CLIENT_SECRET_PREFIX: &str = "socs_";

/// A confidential OIDC client's secret: `socs_` followed by 256 random bits in
/// unpadded base64url. It is shown once; only its digest is stored.
///
/// SHA-256 suffices for the digest, rather than a slow password hash: a 256-bit
/// random value cannot be guessed, so slowing each guess down adds nothing but
/// latency to every token request. The secret is zeroized when dropped.
pub struct ClientSecret(SecretString);
impl ClientSecret {
    /// Generates a secret using operating system randomness.
    ///
    /// # Errors
    /// Preserves failures from the random source.
    pub fn generate() -> Result<Self, CryptoError> {
        let encoded = Zeroizing::new(Secret::generate()?.encoded());
        let mut secret = String::with_capacity(CLIENT_SECRET_PREFIX.len() + encoded.len());
        secret.push_str(CLIENT_SECRET_PREFIX);
        secret.push_str(&encoded);
        Ok(Self(secret.into()))
    }
    /// A secret generated elsewhere, such as by the deployment of the client that
    /// uses it: 256 random bits in unpadded base64url, with or without the prefix.
    ///
    /// # Errors
    /// Rejects other encodings and lengths, so a guessable secret is never stored.
    pub fn parse(value: &str) -> Result<Self, CryptoError> {
        Secret::parse(value.strip_prefix(CLIENT_SECRET_PREFIX).unwrap_or(value))?;
        Ok(Self(value.into()))
    }
    /// The secret, to show the operator once.
    #[must_use]
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }
    /// The digest stored for this secret.
    #[must_use]
    pub fn digest(&self) -> Digest {
        Self::digest_presented(self.expose())
    }
    /// The digest of a secret a client presents, to compare with stored digests.
    /// Secrets that predate the prefix are hashed the same way, so they keep working.
    #[must_use]
    pub fn digest_presented(secret: &str) -> Digest {
        Digest::sha256(secret.as_bytes())
    }
}
impl fmt::Debug for ClientSecret {
    /// Renders a safe diagnostic representation without exposing the secret.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ClientSecret(REDACTED)")
    }
}
