//! Hexadecimal text, as secrets and trace IDs are written.

/// The bytes `text` holds as hexadecimal digits, in either case, or `None` when it
/// has an odd length or a character that is not a hex digit.
///
/// Every character is checked first, then the bytes are written into a buffer of
/// their exact size, so decoding a secret leaves no partial or reallocated copy
/// behind: wrap the result in a zeroizing type.
///
/// ```
/// use sourceorigin_common::hex;
///
/// assert_eq!(hex::decode("00ff7A"), Some(vec![0x00, 0xff, 0x7a]));
/// assert_eq!(hex::decode("abc"), None);
/// // `from_str_radix` would accept a sign; this does not.
/// assert_eq!(hex::decode("+1"), None);
/// ```
#[must_use]
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let (pairs, []) = text.as_bytes().as_chunks::<2>() else {
        return None;
    };
    if !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(
        pairs
            .iter()
            .map(|[high, low]| digit(*high) << 4 | digit(*low))
            .collect(),
    )
}

/// The value of an ASCII hex digit, which [`decode`] has already checked.
const fn digit(byte: u8) -> u8 {
    match byte {
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => byte.wrapping_sub(b'0'),
    }
}
