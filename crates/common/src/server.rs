//! Accounts' identifiers and vocabularies: what auth, its database and its pages
//! share, and the SDKs never read.

use crate::{InvalidValue, protocol};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use uuid::Uuid;

/// A value that identifies a person outside SourceOrigin, such as a provider's subject
/// identifier or a client IP. Storage keeps it apart from the record holding it, so
/// it can be deleted on its own; it then reads back as [`Self::Erased`].
///
/// It serializes as `{"$personal": value}`, or `"$erased"` once deleted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Personal<T> {
    /// The value, while it is kept.
    #[serde(rename = "$personal")]
    Value(T),
    /// The value was deleted or never recorded.
    #[serde(rename = "$erased")]
    Erased,
}

str_enum! {
    /// Supported external authentication providers.
    pub enum Provider {
        /// GitHub OAuth identity.
        GitHub = "github",
        /// Google `OpenID` Connect identity.
        Google = "google",
    }
}

impl Provider {
    /// Its name, as pages show it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::GitHub => "GitHub",
            Self::Google => "Google",
        }
    }
}

uuid_id!(PasskeyId, "Identifier of a stored passkey credential.");
uuid_id!(EventId, "Identifier of a recorded domain event.");
uuid_id!(
    EmailId,
    "Identifier of an email address attached to an account."
);
uuid_id!(
    ClientSecretId,
    "Identifier of one of an OAuth client's secrets."
);
uuid_id!(
    PictureId,
    "Identifier of one upload of an account's profile picture, in its URL: a new picture gets a new one."
);
/// The media types auth reads a profile picture from, as an upload's file input
/// and a provider picture's fetch name them in `accept` and `Accept`; auth
/// recognises the format by content.
pub const PICTURE_TYPES: &str = "image/png,image/jpeg,image/gif";
uuid_id!(
    RecoveryId,
    "Identifier of an account recovery, which its link's token is derived from."
);
uuid_id!(
    NotificationId,
    "Identifier of a notification in an account's inbox."
);
uuid_id!(
    AnnouncementId,
    "Identifier of an announcement administrators sent to accounts, by email and in their settings."
);
uuid_id!(AlertId, "Identifier of an alert administrators are shown.");
/// The W3C Trace Context ID of a request's trace: 16 bytes, not all zero, written
/// as 32 lower-case hex digits. Telemetry assigns one to every request and job run;
/// a recorded event keeps the one of the request that caused it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TraceId([u8; 16]);

impl TraceId {
    /// Wraps a trace ID's bytes, rejecting the all-zero invalid ID.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 16]) -> Option<Self> {
        (bytes != [0; 16]).then_some(Self(bytes))
    }
}

impl fmt::Display for TraceId {
    /// Writes the 32 lower-case hex digits.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0
            .iter()
            .try_for_each(|byte| write!(formatter, "{byte:02x}"))
    }
}

impl FromStr for TraceId {
    type Err = InvalidValue;
    /// Parses exactly 32 lower-case hex digits, not all zero.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let invalid = || InvalidValue::new("TraceId");
        if value.len() != 32
            || !value
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(invalid());
        }
        let mut bytes = [0; 16];
        let digit = |digit: u8| {
            char::from(digit)
                .to_digit(16)
                .and_then(|digit| u8::try_from(digit).ok())
                .ok_or_else(invalid)
        };
        for (byte, [high, low]) in bytes.iter_mut().zip(value.as_bytes().as_chunks().0) {
            *byte = digit(*high)? << 4 | digit(*low)?;
        }
        Self::from_bytes(bytes).ok_or_else(invalid)
    }
}

text_conversions!(TraceId);

__str_enum_sqlx!(TraceId);

str_enum! {
    /// Account availability state in SQL and JSON.
    pub enum AccountStatus {
        /// The account can authenticate.
        Active = "active",
        /// The account is administratively unavailable.
        Disabled = "disabled",
        /// The account is scheduled for deletion. It cannot sign in, but a passkey
        /// can restore it until its grace period ends and it is purged.
        Deleting = "deleting",
    }
}

str_enum! {
    /// Authentication method recorded on a session or token.
    pub enum AuthenticationMethod {
        /// A `WebAuthn` passkey authenticated the user.
        Passkey = "passkey",
        /// An approved external identity provider authenticated the user.
        Federation = "federation",
    }
}

impl AuthenticationMethod {
    /// The `amr` values (RFC 8176) this method stands for. A passkey requires user
    /// verification, so it proves possession and the user's presence: two factors.
    #[must_use]
    pub const fn amr(self) -> &'static [protocol::claims::Amr] {
        match self {
            Self::Passkey => &[
                protocol::claims::Amr::Pop,
                protocol::claims::Amr::Mfa,
                protocol::claims::Amr::User,
            ],
            Self::Federation => &[protocol::claims::Amr::Fed],
        }
    }
}
