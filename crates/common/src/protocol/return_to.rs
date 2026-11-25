//! The path a sign-in sends the browser back to, which auth and the SDKs accept by
//! one rule, so login cannot be turned into an open redirect.
//! `tests/sdk-vectors/return-to` pins it for every SDK.

use super::constants::MAX_RETURN_TO_BYTES;
use std::str::FromStr;
use thiserror::Error;

/// A `return_to` path: same-origin and relative. It starts with one `/` (not `//`
/// or `/\`), is printable ASCII without `\` or `#`, as a `Location` header holds it,
/// and has at most 2048 bytes. Anything else, such as a space or raw UTF-8, must be
/// percent-encoded. Absolute URLs are refused, even the site's own.
///
/// ```
/// use sourceorigin_common::protocol::return_to::{InvalidReturnTo, ReturnTo};
///
/// assert_eq!("/notes?page=2".parse::<ReturnTo>()?.as_str(), "/notes?page=2");
/// assert!("/caf%C3%A9".parse::<ReturnTo>().is_ok());
/// assert_eq!("/café".parse::<ReturnTo>(), Err(InvalidReturnTo::NotPrintableAscii));
/// // Another site, protocol-relative or not, is refused.
/// assert_eq!("https://evil.example".parse::<ReturnTo>(), Err(InvalidReturnTo::NotAPath));
/// assert_eq!("//evil.example".parse::<ReturnTo>(), Err(InvalidReturnTo::ProtocolRelative));
/// assert_eq!("/\\evil.example".parse::<ReturnTo>(), Err(InvalidReturnTo::Backslash));
/// # Ok::<(), InvalidReturnTo>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReturnTo(String);

/// Why a `return_to` is refused. Each message names the rule it breaks, as auth
/// answers it. A path breaking several rules is refused for the first in this
/// order: its length, a backslash, a fragment, a character that is not printable
/// ASCII, a leading `//`, and not starting with `/`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum InvalidReturnTo {
    /// Anything but a path, such as an absolute URL, even one on auth.
    #[error("return_to must be a path on auth, starting with /")]
    NotAPath,
    /// Browsers read a leading `//` as another host.
    #[error("return_to cannot start with //, which browsers read as another host")]
    ProtocolRelative,
    /// Browsers treat `\` like `/`, so it could also reach another host.
    #[error("return_to cannot contain a backslash")]
    Backslash,
    /// Fragments are not kept through the sign-in redirects.
    #[error("return_to cannot contain a # fragment")]
    Fragment,
    /// A space, a control character or raw UTF-8, which would reach `Location`
    /// as it is: anything but printable ASCII must be percent-encoded.
    #[error("return_to must be printable ASCII; percent-encode anything else")]
    NotPrintableAscii,
    /// The path is longer than its holders store.
    #[error("return_to is {0} bytes; at most {MAX_RETURN_TO_BYTES} are allowed")]
    TooLong(usize),
}

impl ReturnTo {
    /// The path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ReturnTo {
    type Err = InvalidReturnTo;
    /// Accepts only same-origin relative paths, checking the rules in the order
    /// [`InvalidReturnTo`] gives.
    fn from_str(path: &str) -> Result<Self, Self::Err> {
        if path.len() > MAX_RETURN_TO_BYTES {
            return Err(InvalidReturnTo::TooLong(path.len()));
        }
        if path.contains('\\') {
            return Err(InvalidReturnTo::Backslash);
        }
        if path.contains('#') {
            return Err(InvalidReturnTo::Fragment);
        }
        if !path.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(InvalidReturnTo::NotPrintableAscii);
        }
        if path.starts_with("//") {
            return Err(InvalidReturnTo::ProtocolRelative);
        }
        if !path.starts_with('/') {
            return Err(InvalidReturnTo::NotAPath);
        }
        Ok(Self(path.to_owned()))
    }
}

impl From<ReturnTo> for String {
    /// The path.
    fn from(path: ReturnTo) -> Self {
        path.0
    }
}

#[cfg(test)]
mod tests {
    //! The rule a refused path is refused for.

    use super::*;

    /// A path breaking several rules is refused for the first, as auth's answer names.
    #[test]
    fn refuses_for_the_first_rule_broken() {
        for (path, rule) in [
            (
                format!("/#\\{}", "a".repeat(MAX_RETURN_TO_BYTES)),
                InvalidReturnTo::TooLong(2051),
            ),
            ("//a#\\".into(), InvalidReturnTo::Backslash),
            ("//a b#".into(), InvalidReturnTo::Fragment),
            ("//\n/evil".into(), InvalidReturnTo::NotPrintableAscii),
            ("/a\u{7f}".into(), InvalidReturnTo::NotPrintableAscii),
            ("//evil.example".into(), InvalidReturnTo::ProtocolRelative),
            ("javascript:alert(1)".into(), InvalidReturnTo::NotAPath),
            (String::new(), InvalidReturnTo::NotAPath),
        ] {
            assert_eq!(path.parse::<ReturnTo>(), Err(rule), "{path:?}");
        }
    }
}
