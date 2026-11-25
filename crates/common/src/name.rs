//! The rule every name a person chooses for others to read follows: an account's
//! display name, a passkey's label and a client's name.

/// Most Unicode characters in a name, which the forms that type one limit too.
pub const MAX_NAME_CHARS: usize = 80;

/// Whether `value` is a name: not blank, at most [`MAX_NAME_CHARS`] characters, and
/// without control characters, which could rewrite the terminal or page showing it.
///
/// ```
/// use sourceorigin_common::is_valid_name;
///
/// assert!(is_valid_name("Notes"));
/// assert!(!is_valid_name("  "));
/// assert!(!is_valid_name("line\nbreak"));
/// assert!(is_valid_name(&"é".repeat(80)));
/// assert!(!is_valid_name(&"é".repeat(81)));
/// ```
#[must_use]
pub fn is_valid_name(value: &str) -> bool {
    !value.trim().is_empty()
        && value.chars().count() <= MAX_NAME_CHARS
        && !value.chars().any(char::is_control)
}
