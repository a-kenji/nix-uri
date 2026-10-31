//! Percent-encoding of a rendered URL.
//!
//! Rendering keeps RFC 3986's unreserved characters (`A-Z`, `a-z`, `0-9`, `-`, `.`, `_`,
//! `~`) and encodes everything else, with a few extra characters kept per component: `:@`
//! in a path segment and `:@/?` in a query key or value.
//!
//! On the decode side the helper is strict: a stray `%` not followed by exactly two hex
//! digits is rejected as `NixUriError::InvalidUrl`.

use std::borrow::Cow;

use percent_encoding::{
    AsciiSet, NON_ALPHANUMERIC, PercentEncode, percent_decode_str, utf8_percent_encode,
};

use crate::error::NixUriError;

/// Everything but RFC 3986's unreserved characters.
const RESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// A query key or value keeps `:@/?` unescaped.
const QUERY: &AsciiSet = &RESERVED.remove(b':').remove(b'@').remove(b'/').remove(b'?');

/// A path segment keeps `:@` unescaped.
const PATH_SEGMENT: &AsciiSet = &RESERVED.remove(b':').remove(b'@');

/// An IP literal keeps its `:` separators.
const IP_LITERAL: &AsciiSet = &RESERVED.remove(b':');

pub(crate) fn encode_query(s: &str) -> PercentEncode<'_> {
    utf8_percent_encode(s, QUERY)
}

/// Encode a fragment, or the user, password or host name of an authority.
pub(crate) fn encode_unreserved(s: &str) -> PercentEncode<'_> {
    utf8_percent_encode(s, RESERVED)
}

/// Encode a single decoded path segment, such as a git-forge owner or a ref.
pub(crate) fn encode_path_segment(s: &str) -> PercentEncode<'_> {
    utf8_percent_encode(s, PATH_SEGMENT)
}

pub(crate) fn encode_ip_literal(s: &str) -> PercentEncode<'_> {
    utf8_percent_encode(s, IP_LITERAL)
}

pub(crate) fn is_unreserved(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~')
}

pub(crate) fn is_sub_delim(c: char) -> bool {
    matches!(
        c,
        '!' | '$' | '&' | '\'' | '(' | ')' | '*' | '+' | ',' | ';' | '='
    )
}

/// A character of RFC 3986's `pchar`, counting the `%` of an escape.
fn is_pchar(c: char) -> bool {
    is_unreserved(c) || is_sub_delim(c) || matches!(c, ':' | '@' | '%')
}

pub(crate) fn is_url_path_char(c: char) -> bool {
    is_pchar(c) || c == '/'
}

/// A character of a user name, password or registered host name.
pub(crate) fn is_userinfo_char(c: char) -> bool {
    is_unreserved(c) || is_sub_delim(c) || matches!(c, ':' | '%')
}

/// A character of a registered host name.
pub(crate) fn is_reg_name_char(c: char) -> bool {
    is_unreserved(c) || is_sub_delim(c) || c == '%'
}

/// A character allowed unescaped in a query.
pub(crate) fn is_query_char(c: char) -> bool {
    is_url_path_char(c) || matches!(c, '?' | '[' | ']' | ' ' | '"')
}

/// Returns [`NixUriError::InvalidUrl`] unless every `%` byte in `s` is followed by
/// exactly two ASCII hex digits.
pub(crate) fn check_percent_escapes(s: &str) -> Result<(), NixUriError> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 3 > bytes.len()
                || !bytes[i + 1].is_ascii_hexdigit()
                || !bytes[i + 2].is_ascii_hexdigit()
            {
                return Err(NixUriError::InvalidUrl(format!(
                    "invalid percent-encoding in '{s}'"
                )));
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    Ok(())
}

pub(crate) fn decode_percent(s: &str) -> Result<Cow<'_, str>, NixUriError> {
    check_percent_escapes(s)?;
    percent_decode_str(s)
        .decode_utf8()
        .map_err(|_| NixUriError::InvalidUrl(format!("invalid utf-8 percent-encoding in '{s}'")))
}

pub(crate) fn mask_non_ascii_escapes(s: &str) -> Cow<'_, str> {
    let bytes = s.as_bytes();
    let is_non_ascii_escape = |i: usize| {
        bytes[i] == b'%'
            && bytes
                .get(i + 1)
                .is_some_and(|b| matches!(b, b'8'..=b'9' | b'A'..=b'F' | b'a'..=b'f'))
            && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit)
    };
    if !(0..bytes.len()).any(is_non_ascii_escape) {
        return Cow::Borrowed(s);
    }
    let mut masked = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if is_non_ascii_escape(i) {
            masked.push_str("%41");
            i += 3;
        } else {
            let c = s[i..].chars().next().unwrap_or_default();
            masked.push(c);
            i += c.len_utf8();
        }
    }
    Cow::Owned(masked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_is_encoded_in_query_value() {
        assert_eq!(encode_query("foo bar").to_string(), "foo%20bar");
    }

    #[test]
    fn slash_is_kept_in_query_value() {
        assert_eq!(encode_query("foo/bar:@?").to_string(), "foo/bar:@?");
    }

    #[test]
    fn slash_is_encoded_in_fragment() {
        assert_eq!(encode_unreserved("foo/bar").to_string(), "foo%2Fbar");
    }

    #[test]
    fn non_ascii_is_encoded() {
        assert_eq!(encode_query("fÖö").to_string(), "f%C3%96%C3%B6");
        assert_eq!(encode_unreserved("fÖö").to_string(), "f%C3%96%C3%B6");
    }

    #[test]
    fn decode_round_trip() {
        let encoded = encode_query("foo bar/baz").to_string();
        assert_eq!(decode_percent(&encoded).unwrap(), "foo bar/baz");
    }

    #[test]
    fn decode_rejects_truncated() {
        assert!(decode_percent("foo%2").is_err());
        assert!(decode_percent("foo%").is_err());
    }

    #[test]
    fn decode_rejects_non_hex() {
        assert!(decode_percent("foo%XY").is_err());
        assert!(decode_percent("foo%2Z").is_err());
    }

    #[test]
    fn path_segment_encodes_sub_delims() {
        // `https://h/a!$&()+,;=:@~b` renders as follows in Nix 2.35.3.
        assert_eq!(
            encode_path_segment("a!$&()+,;=:@~b").to_string(),
            "a%21%24%26%28%29%2B%2C%3B%3D:@~b"
        );
        assert_eq!(encode_path_segment("group/sub").to_string(), "group%2Fsub");
        assert_eq!(encode_path_segment("*").to_string(), "%2A");
    }

    #[test]
    fn decode_passes_valid_full() {
        assert_eq!(decode_percent("foo%20bar").unwrap(), "foo bar");
    }

    #[test]
    fn masks_only_non_ascii_escapes() {
        assert_eq!(
            mask_non_ascii_escapes("a%C3%a9%25%41%zz%8"),
            "a%41%41%25%41%zz%8"
        );
        assert_eq!(mask_non_ascii_escapes("é%20"), "é%20");
    }

    #[test]
    fn unreserved_set_is_rfc_3986_unreserved() {
        for byte in 0..=0x7f_u8 {
            let c = char::from(byte);
            let s = c.to_string();
            assert_eq!(
                encode_unreserved(&s).to_string() == s,
                is_unreserved(c),
                "{c:?}"
            );
        }
    }
}
