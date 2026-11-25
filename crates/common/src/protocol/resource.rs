//! Resource indicators (RFC 8707): the identifier of an API that access tokens
//! are issued for, which they carry as `aud`, and the scopes an API defines.

use super::{
    constants::MAX_IDENTIFIER_BYTES,
    oauth::{Scope, Scopes},
};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use thiserror::Error;
use url::Url;

/// Longest resource identifier, in bytes.
const MAX_RESOURCE_IDENTIFIER_BYTES: usize = 255;

/// The identifier of an API, a protected resource, as a client names it in the
/// `resource` parameter and its access tokens carry it in `aud` (RFC 8707 §2): an
/// absolute `https` URI without a user name, a password, a query or a fragment,
/// of at most 255 bytes.
///
/// It must be written exactly as URL parsing normalizes it, since `aud` is
/// compared as text.
///
/// ```
/// use sourceorigin_common::protocol::resource::{InvalidResourceIdentifier, ResourceIdentifier};
///
/// let notes: ResourceIdentifier = "https://notes.example/api".parse()?;
/// assert_eq!(notes.as_str(), "https://notes.example/api");
/// // A host alone is written with its path, as tokens will carry it.
/// assert!(matches!(
///     "https://notes.example".parse::<ResourceIdentifier>(),
///     Err(InvalidResourceIdentifier::NotCanonical { .. })
/// ));
/// assert!("http://notes.example/".parse::<ResourceIdentifier>().is_err());
/// assert!("https://notes.example/api?v=1".parse::<ResourceIdentifier>().is_err());
/// # Ok::<(), InvalidResourceIdentifier>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ResourceIdentifier(String);

/// Why a [`ResourceIdentifier`] was rejected.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum InvalidResourceIdentifier {
    /// The value is not an absolute URL.
    #[error("resource {resource:?} is not an absolute URL")]
    NotAbsolute {
        /// The rejected value.
        resource: String,
        /// The URL parser's diagnostic.
        #[source]
        source: url::ParseError,
    },
    /// The URL uses another scheme than `https`.
    #[error("resource {resource} must use https, e.g. https://notes.example/api")]
    Insecure {
        /// The rejected value.
        resource: String,
    },
    /// The URL holds a user name or password. The value is omitted: it may hold a
    /// secret.
    #[error("resources must not contain a user name or password")]
    Credentials,
    /// The URL has a query or a fragment (RFC 8707 §2).
    #[error("resource {resource} must not contain a query or a fragment")]
    QueryOrFragment {
        /// The rejected value.
        resource: String,
    },
    /// The URL is longer than 255 bytes.
    #[error("resource {resource} must be at most {MAX_RESOURCE_IDENTIFIER_BYTES} bytes")]
    TooLong {
        /// The rejected value.
        resource: String,
    },
    /// The URL is not written in its normalized form.
    #[error("resource {resource} must be written as {canonical}, as tokens will carry it")]
    NotCanonical {
        /// The rejected value.
        resource: String,
        /// The same URL in its normalized form.
        canonical: String,
    },
}

impl ResourceIdentifier {
    /// The identifier as clients send it and tokens carry it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ResourceIdentifier {
    type Err = InvalidResourceIdentifier;
    /// Applies the rules in [`ResourceIdentifier`]'s description.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let resource = || value.to_owned();
        let url = Url::parse(value).map_err(|source| InvalidResourceIdentifier::NotAbsolute {
            resource: resource(),
            source,
        })?;
        if url.scheme() != "https" {
            return Err(InvalidResourceIdentifier::Insecure {
                resource: resource(),
            });
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(InvalidResourceIdentifier::Credentials);
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(InvalidResourceIdentifier::QueryOrFragment {
                resource: resource(),
            });
        }
        if value.len() > MAX_RESOURCE_IDENTIFIER_BYTES {
            return Err(InvalidResourceIdentifier::TooLong {
                resource: resource(),
            });
        }
        if url.as_str() != value {
            return Err(InvalidResourceIdentifier::NotCanonical {
                resource: resource(),
                canonical: url.into(),
            });
        }
        Ok(Self(value.to_owned()))
    }
}

text_conversions!(ResourceIdentifier);

impl fmt::Display for ResourceIdentifier {
    /// Writes the identifier unchanged.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

crate::__str_enum_sqlx!(ResourceIdentifier);

/// A scope an API defines for itself, such as `notes:read`: an RFC 6749 §3.3
/// scope token of at most 64 characters that is none of auth's own [`Scope`]s,
/// which every client requests whatever the API.
///
/// ```
/// use sourceorigin_common::protocol::resource::{InvalidResourceScope, ResourceScope};
///
/// let read: ResourceScope = "notes:read".parse()?;
/// assert_eq!(read.as_str(), "notes:read");
/// assert_eq!(
///     "openid".parse::<ResourceScope>(),
///     Err(InvalidResourceScope::Reserved("openid".into()))
/// );
/// assert!("two words".parse::<ResourceScope>().is_err());
/// # Ok::<(), InvalidResourceScope>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ResourceScope(String);

/// Why a [`ResourceScope`] was rejected.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum InvalidResourceScope {
    /// The value is not a scope token of at most 64 characters.
    #[error(
        "scope {0:?} must be 1 to 64 printable ASCII characters other than space, '\"' and '\\'"
    )]
    Syntax(String),
    /// The value is one of auth's own scopes.
    #[error("scope {0} is one of Origin Auth's own, which no API defines")]
    Reserved(String),
}

impl ResourceScope {
    /// The scope as requested on the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ResourceScope {
    type Err = InvalidResourceScope;
    /// Accepts scope tokens of at most 64 characters, auth's own aside.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() > MAX_IDENTIFIER_BYTES || !Scopes::is_token(value) {
            return Err(InvalidResourceScope::Syntax(value.to_owned()));
        }
        if value.parse::<Scope>().is_ok() {
            return Err(InvalidResourceScope::Reserved(value.to_owned()));
        }
        Ok(Self(value.to_owned()))
    }
}

text_conversions!(ResourceScope);

impl fmt::Display for ResourceScope {
    /// Writes the scope unchanged.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

crate::__str_enum_sqlx!(ResourceScope);
