//! OAuth client registrations. Each field has a type that validates it when built,
//! so a [`ClientRegistration`](crate::clients::ClientRegistration) cannot hold an
//! invalid value, and the repository, which accepts nothing else, never stores one.
//! Every error names the value it rejected, e.g.
//! `URI http://example.com/cb must use https`.
//!
//! ```
//! use sourceorigin_common::{
//!     ClientId,
//!     clients::{InvalidRedirectUri, RedirectUri},
//! };
//!
//! let id: ClientId = "notes-web".parse()?;
//! let callback: RedirectUri = "https://notes.example/callback".parse()?;
//! assert_eq!(callback.origin(), "https://notes.example");
//! // Plain HTTP is only for loopback IPs, where native apps listen.
//! assert!("http://127.0.0.1:8080/callback".parse::<RedirectUri>().is_ok());
//! assert!(matches!(
//!     "http://notes.example/callback".parse::<RedirectUri>(),
//!     Err(InvalidRedirectUri::Insecure { .. })
//! ));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::{
    ClientId, MAX_NAME_CHARS, UserId, is_shown_text, is_valid_name,
    net::PublicAddress,
    protocol::oauth::{ClientAuthMethod, GrantType, Scope},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{fmt, net::IpAddr, ops::Deref, str::FromStr};
use thiserror::Error;
use url::{Host, Url};

/// Longest description of a resource's scope, in characters.
const MAX_SCOPE_DESCRIPTION_CHARS: usize = 200;
/// Brands only operators may name a client after, since an account's client with
/// one would read as theirs on the consent page. A name containing one, or a
/// look-alike, is refused for clients accounts own (see
/// `DisplayName::ensure_unreserved` in `similarity`). Short common words, such as
/// "apple" or "meta", are left out, since they appear inside ordinary names.
pub const RESERVED_BRANDS: &[&str] = &[
    "sourceorigin",
    "originauth",
    "originaccount",
    "google",
    "microsoft",
    "github",
    "gitlab",
    "facebook",
    "instagram",
    "discord",
    "paypal",
    "amazon",
];
crate::str_enum! {
    /// Whether a client can keep a secret (RFC 6749 §2.1).
    pub enum ClientKind {
        /// Runs where users can read its code, such as a browser or native app.
        /// It authenticates with PKCE alone, and its redirect origins may call the
        /// token and `UserInfo` endpoints from a browser.
        Public = "public",
        /// Runs on a server. It authenticates with a secret auth generates.
        Confidential = "confidential",
    }
}

impl ClientKind {
    /// How a client of this kind authenticates at the token and revocation
    /// endpoints.
    #[must_use]
    pub const fn auth_method(self) -> ClientAuthMethod {
        match self {
            Self::Public => ClientAuthMethod::None,
            Self::Confidential => ClientAuthMethod::ClientSecretBasic,
        }
    }

    /// The scopes a new client of this kind may request unless its registration
    /// names others: `openid`, `profile` and `email`, and `offline_access` for a
    /// confidential client, which the SDKs' login routes always request.
    #[must_use]
    pub const fn default_scopes(self) -> &'static [Scope] {
        match self {
            Self::Public => &[Scope::OpenId, Scope::Profile, Scope::Email],
            Self::Confidential => &[
                Scope::OpenId,
                Scope::Profile,
                Scope::Email,
                Scope::OfflineAccess,
            ],
        }
    }

    /// The grants a new client of this kind may use unless its registration names
    /// others: the authorization code, and refresh tokens for a confidential
    /// client, so the SDKs' login routes keep its sessions.
    #[must_use]
    pub const fn default_grant_types(self) -> &'static [GrantType] {
        match self {
            Self::Public => &[GrantType::AuthorizationCode],
            Self::Confidential => &[GrantType::AuthorizationCode, GrantType::RefreshToken],
        }
    }

    /// The grants a client of this kind is registered with, from `grants`: at
    /// least one, each once.
    ///
    /// # Errors
    /// Fails if there are none, or if a public client is given
    /// [`GrantType::ClientCredentials`]: it has no credentials of its own to
    /// present, so anyone knowing its ID would get its tokens.
    pub fn grant_types(
        self,
        grants: impl IntoIterator<Item = GrantType>,
    ) -> Result<GrantTypes, InvalidGrantTypes> {
        let grants = GrantTypes::new(grants)?;
        match self {
            Self::Public if grants.contains(&GrantType::ClientCredentials) => {
                Err(InvalidGrantTypes::Confidential)
            }
            Self::Public | Self::Confidential => Ok(grants),
        }
    }
}

/// Grant types a client of its kind may not be registered with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum InvalidGrantTypes {
    /// There are none, or more than there are grants.
    #[error(transparent)]
    Count(#[from] InvalidCount),
    /// A public client was given the client credentials grant.
    #[error(
        "only an app with a backend, which keeps a secret, may use the client_credentials grant"
    )]
    Confidential,
}

/// Who manages a client, fixed when it is registered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientOwner {
    /// An operator, with the `client` command. No account manages it, and no
    /// account's own client list shows it.
    Operator,
    /// The account that registered it, which alone may change it as its owner.
    Account(UserId),
}

crate::str_enum! {
    /// How a client was registered, stored beside its owner.
    pub enum CreatedVia {
        /// By an operator, with the `client` command.
        Cli = "cli",
        /// By its owner, signed in to auth.
        Account = "account",
    }
}

/// Whether a client signs users in, and why not. The first that applies of a
/// deletion, a suspension, then the client being enabled or not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientStatus {
    /// It was deleted. It stays listed, disabled, so its ID is never reused.
    Deleted {
        /// When.
        at: DateTime<Utc>,
    },
    /// An operator suspended it, and it stays disabled until they lift that: its
    /// owner cannot enable it.
    Suspended {
        /// When.
        at: DateTime<Utc>,
    },
    /// Its owner or an operator disabled it.
    Disabled,
    /// It signs users in.
    Enabled,
}

impl ClientStatus {
    /// The status of a client `enabled` or not, suspended at `suspended_at` and
    /// deleted at `deleted_at`, if ever.
    #[must_use]
    pub const fn of(
        enabled: bool,
        suspended_at: Option<DateTime<Utc>>,
        deleted_at: Option<DateTime<Utc>>,
    ) -> Self {
        match (deleted_at, suspended_at, enabled) {
            (Some(at), _, _) => Self::Deleted { at },
            (None, Some(at), _) => Self::Suspended { at },
            (None, None, false) => Self::Disabled,
            (None, None, true) => Self::Enabled,
        }
    }

    /// The status in one word: `deleted`, `suspended`, `disabled` or `enabled`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Deleted { .. } => "deleted",
            Self::Suspended { .. } => "suspended",
            Self::Disabled => "disabled",
            Self::Enabled => "enabled",
        }
    }
}

impl fmt::Display for ClientStatus {
    /// Writes [`ClientStatus::as_str`].
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl ClientOwner {
    /// The managing account, if any.
    #[must_use]
    pub const fn user_id(self) -> Option<UserId> {
        match self {
            Self::Operator => None,
            Self::Account(user_id) => Some(user_id),
        }
    }

    /// How a client with this owner was registered.
    #[must_use]
    pub const fn created_via(self) -> CreatedVia {
        match self {
            Self::Operator => CreatedVia::Cli,
            Self::Account(_) => CreatedVia::Account,
        }
    }
}

crate::open_str_enum! {
    /// A setting of a [`ClientRegistration`], named as its field is. Errors name
    /// the field they reject, and `ClientUpdated` events the fields that changed.
    /// Open, since a newer release adds settings (branding, #45) to events this
    /// release still reads.
    pub enum ClientField {
        /// [`ClientRegistration::display_name`].
        DisplayName = "display_name",
        /// [`ClientRegistration::redirect_uris`].
        RedirectUris = "redirect_uris",
        /// [`ClientRegistration::post_logout_uris`].
        PostLogoutUris = "post_logout_uris",
        /// [`ClientRegistration::allowed_scopes`].
        AllowedScopes = "allowed_scopes",
        /// [`ClientRegistration::grant_types`].
        GrantTypes = "grant_types",
        /// [`ClientRegistration::trusted`].
        Trusted = "trusted",
        /// [`ClientRegistration::require_pushed_authorization_requests`].
        RequirePushedAuthorizationRequests = "require_pushed_authorization_requests",
        /// [`ClientRegistration::backchannel_logout_uri`].
        BackchannelLogoutUri = "backchannel_logout_uri",
    }
}

impl ClientField {
    /// Whether changing this field changes where auth may send the browser, which
    /// would let whoever changes it collect authorization codes.
    #[must_use]
    pub const fn redirects(&self) -> bool {
        match self {
            Self::RedirectUris | Self::PostLogoutUris => true,
            Self::DisplayName
            | Self::AllowedScopes
            | Self::GrantTypes
            | Self::Trusted
            | Self::RequirePushedAuthorizationRequests
            | Self::BackchannelLogoutUri
            | Self::Other(_) => false,
        }
    }
}

/// The name a consent screen shows, by the rule of every name ([`is_valid_name`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayName(String);

/// A display name that is blank, longer than [`MAX_NAME_CHARS`] characters, or has
/// control characters.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error(
    "display name {0:?} must be nonblank, at most {MAX_NAME_CHARS} characters and have no control characters"
)]
pub struct InvalidDisplayName(String);

/// A display name containing a brand of [`RESERVED_BRANDS`], or a look-alike.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("names containing \"{0}\", or a look-alike, are reserved")]
pub struct ReservedDisplayName(pub(crate) &'static str);

impl DisplayName {
    /// The name as shown.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for DisplayName {
    type Err = InvalidDisplayName;
    /// Accepts names [`is_valid_name`] accepts.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if !is_valid_name(value) {
            return Err(InvalidDisplayName(value.to_owned()));
        }
        Ok(Self(value.to_owned()))
    }
}

/// What a scope of a resource lets a client do, as the consent page says it to
/// the user: nonblank, at most 200 characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeDescription(String);

/// A scope description that is blank or longer than 200 characters.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error(
    "scope description {0:?} must be nonblank, at most 200 characters and have no control characters"
)]
pub struct InvalidScopeDescription(String);

impl ScopeDescription {
    /// The description as shown.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ScopeDescription {
    type Err = InvalidScopeDescription;
    /// Accepts nonblank descriptions of at most 200 characters, without control
    /// characters.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if !is_shown_text(value, MAX_SCOPE_DESCRIPTION_CHARS) {
            return Err(InvalidScopeDescription(value.to_owned()));
        }
        Ok(Self(value.to_owned()))
    }
}

/// A URI of the client that auth sends the browser to: an authorization callback or
/// a post-logout page. It uses HTTPS, or HTTP on a loopback IP for native apps (RFC
/// 8252 §7.3) and development, and has no user name, password or fragment.
///
/// It must be written exactly as URL parsing normalizes it, e.g.
/// `https://app.example/` rather than `https://APP.example`: clients send it
/// verbatim, and auth compares it as a string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RedirectUri(Url);

/// Why a [`RedirectUri`] or a [`BackchannelLogoutUri`] was rejected. The messages say
/// "URI" alone, since post-logout pages are checked as callbacks are.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum InvalidRedirectUri {
    /// The value is not an absolute URL.
    #[error("URI {uri:?} is not an absolute URL")]
    NotAbsolute {
        /// The rejected value.
        uri: String,
        /// The URL parser's diagnostic.
        #[source]
        source: url::ParseError,
    },
    /// The URL uses another scheme, or HTTP on a host other than a loopback IP.
    #[error(
        "URI {uri} must use https; http is allowed only on a loopback IP, \
         e.g. http://127.0.0.1:8080/callback"
    )]
    Insecure {
        /// The rejected value.
        uri: String,
    },
    /// A back-channel logout URI that auth can't deliver to: not HTTPS, or an IP
    /// address that isn't public.
    #[error("URI {uri} must use https with a public address, which auth's server can reach")]
    Unreachable {
        /// The rejected value.
        uri: String,
    },
    /// The URL holds a user name or password. The value is omitted: it may hold a secret.
    #[error("URIs must not contain a user name or password")]
    Credentials,
    /// The URL has a fragment, which authorization responses cannot carry (RFC 6749 §3.1.2).
    #[error("URI {uri} must not contain a fragment")]
    Fragment {
        /// The rejected value.
        uri: String,
    },
    /// The URL is not written in its normalized form.
    #[error("URI {uri} must be written as {canonical}, as clients will send it")]
    NotCanonical {
        /// The rejected value.
        uri: String,
        /// The same URL in its normalized form.
        canonical: String,
    },
}

impl RedirectUri {
    /// The URI as registered and as clients send it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// The URI's origin as browsers send it in an `Origin` header, e.g.
    /// `https://app.example`, which may call the token endpoint for a public client.
    #[must_use]
    pub fn origin(&self) -> String {
        self.0.origin().ascii_serialization()
    }
}

impl FromStr for RedirectUri {
    type Err = InvalidRedirectUri;
    /// Applies the rules in [`RedirectUri`]'s description.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let uri = || value.to_owned();
        let url = Url::parse(value)
            .map_err(|source| InvalidRedirectUri::NotAbsolute { uri: uri(), source })?;
        let loopback = match url.host() {
            Some(Host::Ipv4(ip)) => ip.is_loopback(),
            Some(Host::Ipv6(ip)) => ip.is_loopback(),
            Some(Host::Domain(_)) | None => false,
        };
        if !(url.scheme() == "https" || (url.scheme() == "http" && loopback)) {
            return Err(InvalidRedirectUri::Insecure { uri: uri() });
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(InvalidRedirectUri::Credentials);
        }
        if url.fragment().is_some() {
            return Err(InvalidRedirectUri::Fragment { uri: uri() });
        }
        if url.as_str() != value {
            return Err(InvalidRedirectUri::NotCanonical {
                uri: uri(),
                canonical: url.into(),
            });
        }
        Ok(Self(url))
    }
}

impl fmt::Display for RedirectUri {
    /// Writes the URI as registered.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Where auth posts a client's back-channel logout tokens, server to server: a
/// [`RedirectUri`] that auth can reach, so HTTPS, and, when its host is an IP
/// address, a public one. A name is checked when auth delivers to it, since what
/// it resolves to can change.
///
/// ```
/// use sourceorigin_common::clients::BackchannelLogoutUri;
///
/// assert!("https://notes.example/logout".parse::<BackchannelLogoutUri>().is_ok());
/// // No server reaches the client's own loopback or private network.
/// assert!("http://127.0.0.1:8080/logout".parse::<BackchannelLogoutUri>().is_err());
/// assert!("https://10.0.0.7/logout".parse::<BackchannelLogoutUri>().is_err());
/// // Nor shared or reserved space, which deliveries refuse too.
/// assert!("https://100.64.0.1/logout".parse::<BackchannelLogoutUri>().is_err());
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackchannelLogoutUri(RedirectUri);

impl BackchannelLogoutUri {
    /// The URI as registered.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl FromStr for BackchannelLogoutUri {
    type Err = InvalidRedirectUri;
    /// Applies [`RedirectUri`]'s rules, then this type's.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let uri: RedirectUri = value.parse()?;
        let address = match uri.0.host() {
            Some(Host::Ipv4(v4)) => Some(IpAddr::V4(v4)),
            Some(Host::Ipv6(v6)) => Some(IpAddr::V6(v6)),
            Some(Host::Domain(_)) | None => None,
        };
        if uri.0.scheme() != "https" || address.is_some_and(|address| !address.is_public()) {
            return Err(InvalidRedirectUri::Unreachable {
                uri: value.to_owned(),
            });
        }
        Ok(Self(uri))
    }
}

impl fmt::Display for BackchannelLogoutUri {
    /// Writes the URI as registered.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// An origin whose pages may sign users in with their auth passkeys, as a `WebAuthn`
/// related origin (`WebAuthn` Level 3 §5.11): HTTPS on a domain name, written exactly
/// as browsers send it in an `Origin` header.
///
/// ```
/// use sourceorigin_common::clients::RelatedOrigin;
///
/// let origin: RelatedOrigin = "https://notes.example".parse()?;
/// assert_eq!(origin.host(), "notes.example");
/// // A trailing slash, a path or plain HTTP is not an origin browsers send.
/// assert!("https://notes.example/".parse::<RelatedOrigin>().is_err());
/// assert!("http://notes.example".parse::<RelatedOrigin>().is_err());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RelatedOrigin(Box<Url>);

/// Why a related origin was rejected.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum InvalidRelatedOrigin {
    /// The value is not an absolute URL.
    #[error("origin {origin:?} is not an absolute URL")]
    NotAbsolute {
        /// The rejected value.
        origin: String,
        /// The URL parser's diagnostic.
        #[source]
        source: url::ParseError,
    },
    /// The URL uses another scheme, or its host is an IP address.
    #[error("origin {origin} must use https on a domain name, e.g. https://notes.example")]
    Insecure {
        /// The rejected value.
        origin: String,
    },
    /// The URL holds a user name or password. The value is omitted: it may hold a secret.
    #[error("origins must not contain a user name or password")]
    Credentials,
    /// The value is not written as browsers send the origin: it has a path, a
    /// query, a default port or upper case letters.
    #[error("origin {origin} must be written as {canonical}, as browsers send it")]
    NotCanonical {
        /// The rejected value.
        origin: String,
        /// The origin as browsers send it.
        canonical: String,
    },
}

impl RelatedOrigin {
    /// The origin as browsers send it, e.g. `https://notes.example`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str().trim_end_matches('/')
    }

    /// The origin's domain name, e.g. `notes.example`.
    #[must_use]
    pub fn host(&self) -> &str {
        self.0.host_str().unwrap_or_default()
    }

    /// The origin as a URL, e.g. `https://notes.example/`.
    #[must_use]
    pub fn url(&self) -> &Url {
        &self.0
    }
}

impl FromStr for RelatedOrigin {
    type Err = InvalidRelatedOrigin;
    /// Applies the rules in [`RelatedOrigin`]'s description.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let origin = || value.to_owned();
        let url = Url::parse(value).map_err(|source| InvalidRelatedOrigin::NotAbsolute {
            origin: origin(),
            source,
        })?;
        if url.scheme() != "https" || !matches!(url.host(), Some(Host::Domain(_))) {
            return Err(InvalidRelatedOrigin::Insecure { origin: origin() });
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(InvalidRelatedOrigin::Credentials);
        }
        let canonical = url.origin().ascii_serialization();
        if canonical != value {
            return Err(InvalidRelatedOrigin::NotCanonical {
                origin: origin(),
                canonical,
            });
        }
        Ok(Self(Box::new(url)))
    }
}

text_conversions!(RelatedOrigin);

crate::__str_enum_sqlx!(RelatedOrigin);

impl fmt::Display for RelatedOrigin {
    /// Writes the origin as browsers send it.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Distinct values, at least `MIN` and at most `MAX` of them, in the order given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundedVec<T, const MIN: usize, const MAX: usize>(Vec<T>);

/// A list with too few or too many distinct values. The caller names the field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[error("must list {min} to {max} distinct values, not {found}")]
pub struct InvalidCount {
    /// The fewest values allowed.
    pub min: usize,
    /// The most values allowed.
    pub max: usize,
    /// How many distinct values were given.
    pub found: usize,
}

impl<T: PartialEq, const MIN: usize, const MAX: usize> BoundedVec<T, MIN, MAX> {
    /// Drops repeated values, keeping the first, then checks the count.
    ///
    /// # Errors
    /// Returns [`InvalidCount`] for fewer than `MIN` or more than `MAX` distinct values.
    pub fn new(values: impl IntoIterator<Item = T>) -> Result<Self, InvalidCount> {
        const { assert!(MIN <= MAX, "a BoundedVec needs MIN <= MAX") };
        let mut distinct = Vec::new();
        for value in values {
            if !distinct.contains(&value) {
                distinct.push(value);
            }
        }
        if !(MIN..=MAX).contains(&distinct.len()) {
            return Err(InvalidCount {
                min: MIN,
                max: MAX,
                found: distinct.len(),
            });
        }
        Ok(Self(distinct))
    }
}

impl<T, const MIN: usize, const MAX: usize> Deref for BoundedVec<T, MIN, MAX> {
    type Target = [T];
    /// Reads the values as a slice.
    fn deref(&self) -> &[T] {
        &self.0
    }
}

/// 1 to 16 authorization callbacks.
pub type RedirectUris = BoundedVec<RedirectUri, 1, 16>;
/// Up to 16 pages the browser may return to after logout.
pub type PostLogoutUris = BoundedVec<RedirectUri, 0, 16>;
/// The scopes auth itself defines that a client may request: at least one, each
/// once. An API's own scopes belong to its resource, which says which of them
/// each client may request.
pub type AllowedScopes = BoundedVec<Scope, 1, { Scope::ALL.len() }>;
/// The grants a client may use at the token endpoint, which RFC 7591 §2 calls its
/// `grant_types`: at least one, each once.
pub type GrantTypes = BoundedVec<GrantType, 1, { GrantType::ALL.len() }>;

/// Everything an operator decides about a client. Every field is valid by
/// construction, so storing one needs no further checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientRegistration {
    /// The client's identifier.
    pub id: ClientId,
    /// The name consent screens show.
    pub display_name: DisplayName,
    /// Public, or confidential with a secret.
    pub kind: ClientKind,
    /// Where authorization responses may be sent, matched exactly.
    pub redirect_uris: RedirectUris,
    /// Where the browser may return after logout, matched exactly.
    pub post_logout_uris: PostLogoutUris,
    /// The scopes auth defines that the client may request.
    pub allowed_scopes: AllowedScopes,
    /// The grants the client may use at the token endpoint. It receives refresh
    /// tokens, with `offline_access`, only with [`GrantType::RefreshToken`].
    pub grant_types: GrantTypes,
    /// Whether the client skips the consent screen. Only for first-party clients.
    pub trusted: bool,
    /// Whether the client must push its authorization requests (RFC 9126 §6), so
    /// none of its parameters travel through the browser.
    pub require_pushed_authorization_requests: bool,
    /// Where auth posts a logout token when a session the client holds tokens from
    /// ends (OIDC Back-Channel Logout 1.0).
    pub backchannel_logout_uri: Option<BackchannelLogoutUri>,
}

/// Focused tests for each field's rules.
#[cfg(test)]
mod tests;
