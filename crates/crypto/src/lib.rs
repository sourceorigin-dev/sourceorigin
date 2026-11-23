//! Auditable cryptographic primitives for authentication, shared by auth and the
//! Rust SDK; no browser or database policy.
//!
//! With no features, this crate is what the SDK builds on: random secrets,
//! digests, PKCE challenges, RS256 verification and sealing. The `server` feature
//! adds what only auth uses: RS256 signing and key sets, client secrets, the PKCE
//! verifier check, the key-encryption key, Ed25519 signed notes and SCRAM verifiers.
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(clippy::print_stdout, clippy::print_stderr)]
/// The one RS256 verifier auth and the SDK share, and compact JWS splitting.
mod jws;
/// Ed25519 signed notes, for audit checkpoints.
#[cfg(feature = "server")]
mod note;
/// Client secrets, as auth checks them.
#[cfg(feature = "server")]
mod oauth;
/// PKCE S256 challenges, and auth's check of a verifier.
mod pkce;
/// SCRAM-SHA-256 password verifiers, for database roles.
#[cfg(feature = "server")]
mod scram;
/// AES-256-GCM sealing of private keys at rest, and of the SDK's cookies.
mod sealing;
/// RSA signing and verification of compact JWTs, by key set.
#[cfg(feature = "server")]
mod signing;
pub use jws::{Compact, KeyDefect, Malformed, Rs256VerifyingKey, Split, VerifyError};
#[cfg(feature = "server")]
pub use note::{NoteKey, NoteVerifier, NoteVerifiers};
#[cfg(feature = "server")]
pub use oauth::ClientSecret;
pub use pkce::Pkce;
#[cfg(feature = "server")]
pub use scram::ScramVerifier;
#[cfg(feature = "server")]
pub use sealing::KeyEncryptionKey;
pub use sealing::SealingKey;
#[cfg(feature = "server")]
pub use signing::{PublicKey, Rs256Key, SigningKeySet};

use aws_lc_rs::{
    constant_time, digest, error::KeyRejected, error::Unspecified, hmac, rand as random,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use secrecy::{ExposeSecret, SecretBox, zeroize::Zeroizing};
use sourceorigin_common::protocol::jwt::InvalidKeyId;
use sourceorigin_common::protocol::registered::ClaimsError;
use std::fmt;
use thiserror::Error;

/// Byte length of 256-bit secrets and SHA-256 results.
const SECRET_BYTES: usize = 32;

/// Cryptographic failures, preserving provider diagnostics without secret values.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CryptoError {
    /// The operating system could not supply random bytes.
    #[error("operating system randomness unavailable")]
    Random(#[source] Unspecified),
    /// The cryptographic provider failed.
    #[error("cryptographic provider failed")]
    Provider(#[from] Unspecified),
    /// The cryptographic provider rejected a key, e.g. as malformed.
    #[error("cryptographic provider rejected the key")]
    Key(#[from] KeyRejected),
    /// A base64url value is malformed.
    #[error("invalid unpadded base64url encoding")]
    Encoding(#[from] base64::DecodeError),
    /// A decoded secret has the wrong size.
    #[error("secret must contain exactly 32 bytes")]
    SecretLength,
    /// A PEM private key has no PKCS#8 block.
    #[error("private key PEM must hold a PRIVATE KEY block")]
    Pem,
    /// A public key that every SDK would refuse to verify with.
    #[error("signing key cannot verify signatures")]
    UnusableKey(#[source] KeyDefect),
    /// The key identifier cannot safely appear in JWT headers.
    #[error("signing key ID must contain 1 to 128 characters without whitespace")]
    KeyId(#[source] InvalidKeyId),
    /// A token's compact framing is malformed.
    #[error("JWT framing is invalid")]
    MalformedToken(#[source] Malformed),
    /// A JWT segment contains malformed JSON.
    #[error("invalid JWT JSON")]
    Json(#[from] serde_json::Error),
    /// A token's header names another type or algorithm than the one expected.
    #[error("JWT type or algorithm is not the one expected")]
    UnexpectedHeader,
    /// A JWT signature did not verify.
    #[error("JWT signature is invalid")]
    Signature,
    /// A signed JWT's claims are missing or have the wrong type.
    #[error("JWT claims are invalid")]
    InvalidClaims(#[source] serde_json::Error),
    /// A signed JWT's issuer or times fail the shared check, which names the claim.
    #[error(transparent)]
    Claims(#[from] ClaimsError),
    /// A JWT names a key outside the key set: unknown, revoked or deleted.
    #[error("JWT names a key outside the key set")]
    UnknownKey,
    /// A key set would not publish its signing key, or names a key twice.
    #[error("key set must publish its signing key and name each key once")]
    KeySet,
    /// A sealed value was made with another key or associated data, or modified.
    #[error("sealed value cannot be opened with this key-encryption key")]
    Unsealable,
    /// The PKCE verifier violates RFC 7636.
    #[error("PKCE verifier must contain 43 to 128 unreserved ASCII characters")]
    PkceVerifier,
    /// The PKCE verifier does not match its challenge.
    #[error("PKCE S256 challenge does not match the verifier")]
    PkceMismatch,
    /// A note key's name is empty or contains whitespace or `+`.
    #[error("note key name must be nonempty, without whitespace or +")]
    NoteKeyName,
    /// An Ed25519 seed is not 32 hex-encoded bytes.
    #[error("Ed25519 seed must be 64 hexadecimal characters (32 bytes)")]
    NoteKeySeed,
    /// A verifier key is malformed, not Ed25519, or has the wrong key ID.
    #[error("note verifier key must be <name>+<hex key ID>+<base64 Ed25519 key>")]
    NoteVerifierKey,
    /// A verifier key's base64 is malformed.
    #[error("note verifier key is not valid base64")]
    NoteVerifierKeyEncoding(#[source] base64::DecodeError),
    /// Note text does not end with a newline, or holds another control character.
    #[error("note text must end with a newline and contain no other control characters")]
    NoteText,
    /// A note has no signature block, or a malformed signature line.
    #[error("signed note is malformed")]
    NoteFormat,
    /// A signature line's base64 is malformed.
    #[error("signed note's signature is not valid base64")]
    NoteSignatureEncoding(#[source] base64::DecodeError),
    /// A signature by a known key does not verify.
    #[error("note signature is invalid")]
    NoteSignature,
    /// No known key signed the note.
    #[error("note has no signature by a known key")]
    NoteUnverified,
}

/// `N` bytes of operating system randomness.
///
/// # Errors
/// Preserves failures from the random source.
pub(crate) fn random_bytes<const N: usize>() -> Result<[u8; N], CryptoError> {
    let mut bytes = [0; N];
    random::fill(&mut bytes).map_err(CryptoError::Random)?;
    Ok(bytes)
}

/// A random 256-bit secret, zeroized when dropped, with redacted diagnostics.
pub struct Secret(SecretBox<[u8; SECRET_BYTES]>);
impl Clone for Secret {
    /// Copies the secret into a new zeroizing box.
    fn clone(&self) -> Self {
        Self::from_bytes(self.as_bytes())
    }
}
impl Secret {
    /// Boxes `bytes`, copying them only into the box.
    fn from_bytes(bytes: &[u8; SECRET_BYTES]) -> Self {
        Self(SecretBox::init_with_mut(|boxed| *boxed = *bytes))
    }
    /// Generates a secret using operating system randomness.
    ///
    /// # Errors
    /// Preserves failures from the random source.
    pub fn generate() -> Result<Self, CryptoError> {
        Ok(Self::from_bytes(&Zeroizing::new(random_bytes()?)))
    }
    /// Derives a domain-separated secret key with HMAC-SHA256.
    #[must_use]
    pub fn derive(key: &[u8], context: &[u8]) -> Self {
        Self::from_bytes(&Digest::hmac(key, context).0)
    }
    /// Parses a canonical, unpadded base64url secret.
    ///
    /// # Errors
    /// Rejects invalid encoding and lengths other than 32 bytes.
    pub fn parse(value: &str) -> Result<Self, CryptoError> {
        let bytes = Zeroizing::new(URL_SAFE_NO_PAD.decode(value)?);
        #[expect(
            clippy::map_err_ignore,
            reason = "`TryFromSliceError` carries nothing beyond the length mismatch `SecretLength` names"
        )]
        let bytes: &[u8; SECRET_BYTES] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| CryptoError::SecretLength)?;
        Ok(Self::from_bytes(bytes))
    }
    /// Exposes bytes only when required by a cryptographic operation.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; SECRET_BYTES] {
        self.0.expose_secret()
    }
    /// Encodes the secret for transport.
    #[must_use]
    pub fn encoded(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.as_bytes())
    }
    /// Hashes the secret for storage.
    #[must_use]
    pub fn digest(&self) -> Digest {
        Digest::sha256(self.as_bytes())
    }
}
impl fmt::Debug for Secret {
    /// Renders a safe diagnostic representation without exposing secrets.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret(REDACTED)")
    }
}

/// A SHA-256 digest or HMAC result, always exactly 32 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Digest([u8; SECRET_BYTES]);
impl Digest {
    /// Byte length of every digest.
    pub const LENGTH: usize = SECRET_BYTES;
    /// Computes SHA-256 over arbitrary bytes.
    #[must_use]
    pub fn sha256(value: &[u8]) -> Self {
        Self::from_output(digest::digest(&digest::SHA256, value).as_ref())
    }
    /// Computes HMAC-SHA256 using the supplied key.
    #[must_use]
    pub fn hmac(key: &[u8], value: &[u8]) -> Self {
        let key = hmac::Key::new(hmac::HMAC_SHA256, key);
        Self::from_output(hmac::sign(&key, value).as_ref())
    }
    /// Copies a SHA-256 or HMAC-SHA256 output, always [`Self::LENGTH`] bytes.
    fn from_output(output: &[u8]) -> Self {
        let mut bytes = [0; SECRET_BYTES];
        bytes.copy_from_slice(output);
        Self(bytes)
    }
    /// Exposes the digest for persistence or comparison.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; SECRET_BYTES] {
        &self.0
    }
    /// Encodes the digest with unpadded base64url.
    #[must_use]
    pub fn encoded(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0)
    }
    /// Compares equal-length byte strings in constant time; length is public.
    #[must_use]
    pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
        constant_time::verify_slices_are_equal(left, right).is_ok()
    }
}

/// Focused tests for this module and its security contracts.
#[cfg(test)]
mod tests;
