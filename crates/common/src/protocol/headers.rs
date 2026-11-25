//! The security and cache headers every response of SourceOrigin's servers
//! carries, auth's and sourceorigin.dev's alike, by name and value. Each server
//! adds its own Content-Security-Policy and chooses which value of
//! `Cache-Control` a response gets.

use super::isolation;
use std::time::Duration;

/// How much of a page's address its links and requests send on, as
/// `Referrer-Policy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferrerPolicy {
    /// `no-referrer`: nothing, so navigation never leaks an authentication URL.
    None,
    /// `strict-origin-when-cross-origin`: only the origin to other sites, so a
    /// link to an app carries no path of the page it is on.
    Origin,
}

impl ReferrerPolicy {
    /// The header's value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "no-referrer",
            Self::Origin => "strict-origin-when-cross-origin",
        }
    }
}

/// The security headers of a response, by name and value: the
/// [`isolation`] headers, other sites embedding or reading it only if it is
/// `public`, then no MIME sniffing, the `referrer` policy, no framing, and
/// HTTPS for a year on later visits.
///
/// ```
/// use sourceorigin_common::protocol::headers::{self, ReferrerPolicy};
///
/// let sent = headers::security(false, ReferrerPolicy::None);
/// assert!(sent.contains(&("referrer-policy", "no-referrer")));
/// assert!(sent.contains(&("cross-origin-resource-policy", "same-origin")));
/// ```
#[must_use]
pub const fn security(public: bool, referrer: ReferrerPolicy) -> [(&'static str, &'static str); 7] {
    let [opener, resource, permissions] = isolation::headers(public);
    [
        opener,
        resource,
        permissions,
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", referrer.as_str()),
        ("x-frame-options", "DENY"),
        ("strict-transport-security", "max-age=31536000"),
    ]
}

/// `Cache-Control` of what no cache may keep.
pub const CACHE_NO_STORE: &str = "no-store, no-transform";

/// `Cache-Control` of what never changes at its URL, such as a file named by its
/// content: any cache may keep it for a year.
pub const CACHE_IMMUTABLE: &str = "public, max-age=31536000, immutable, no-transform";

/// `Cache-Control` of what any cache may keep for `max_age`, to the second.
#[must_use]
pub fn cache_public(max_age: Duration) -> String {
    format!("public, max-age={}, no-transform", max_age.as_secs())
}
