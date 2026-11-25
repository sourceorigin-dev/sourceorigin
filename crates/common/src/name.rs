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
    is_shown_text(value, MAX_NAME_CHARS)
}

/// Whether `value` is text a page or a terminal shows, of at most `max`
/// characters: not blank, and without control characters, which could rewrite
/// what shows it. Names follow it, and so does a longer text such as the
/// description of a resource's scope.
///
/// ```
/// use sourceorigin_common::is_shown_text;
///
/// assert!(is_shown_text("Read your notes", 200));
/// assert!(!is_shown_text(" ", 200));
/// assert!(!is_shown_text(&"é".repeat(201), 200));
/// ```
#[must_use]
pub fn is_shown_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.chars().count() <= max && !value.chars().any(char::is_control)
}
