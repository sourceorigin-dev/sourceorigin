//! The identifier of a registered OAuth client, which tokens carry and the SDKs read.

use crate::protocol::constants::MAX_IDENTIFIER_BYTES;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use thiserror::Error;

/// A registered OAuth client's identifier: 1 to 64 ASCII letters, digits, `.`, `_`
/// or `-`. It is fixed at registration and appears in tokens as `client_id`, `azp`
/// and `aud`, so no other client may ever take it, even after a deletion.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ClientId(String);

/// A client ID outside `[A-Za-z0-9._-]{1,64}`.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("client ID {0:?} must be 1 to 64 characters from A-Z, a-z, 0-9, '.', '_' and '-'")]
pub struct InvalidClientId(String);

impl ClientId {
    /// The identifier as sent on the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(feature = "server")]
impl ClientId {
    /// Starts every client ID auth generates.
    const GENERATED_ID_PREFIX: &str = "c_";
    /// Characters after [`Self::GENERATED_ID_PREFIX`], 5 random bits each.
    const GENERATED_ID_CHARS: usize = 20;
    /// The RFC 4648 base32 alphabet, lowercased so generated IDs read well in URLs.
    const BASE32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

    /// The ID auth gives a client an account registers: `c_` and 20 base32
    /// characters, one per byte of `random`, 100 bits in all. Only operators and
    /// administrators choose IDs, so an account can never claim a name such as
    /// `sourceorigin-admin`.
    #[must_use]
    pub fn generated(random: &[u8; 32]) -> Self {
        let suffix = random
            .iter()
            .take(Self::GENERATED_ID_CHARS)
            .map(|byte| char::from(Self::BASE32[usize::from(byte & 0x1f)]));
        Self(Self::GENERATED_ID_PREFIX.chars().chain(suffix).collect())
    }
}

impl FromStr for ClientId {
    type Err = InvalidClientId;
    /// Accepts `[A-Za-z0-9._-]{1,64}`.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let valid = (1..=MAX_IDENTIFIER_BYTES).contains(&value.len())
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(InvalidClientId(value.to_owned()))
        }
    }
}

text_conversions!(ClientId);

impl fmt::Display for ClientId {
    /// Writes the identifier unchanged.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

crate::__str_enum_sqlx!(ClientId);
