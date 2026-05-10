//! Percent-encoding helpers for query values and the fragment.
//!
//! Matches Nix's percent-encoding for query strings and fragments so the
//! Display output round-trips through Nix byte-for-byte.
//!
//! Two `AsciiSet`s are exposed:
//!
//! - [`QUERY_VALUE`]: the RFC 3986 unreserved set plus `:@/?` (the
//!   characters Nix leaves unescaped inside a query). Used for both query
//!   keys and query values.
//! - [`FRAGMENT`]: the bare unreserved set, so `:@/?` are encoded too.
//!
//! On the decode side the helper is strict: a stray `%` not followed by
//! exactly two hex digits is rejected as `NixUriError::InvalidUrl`. The
//! `percent_encoding` crate's [`percent_decode_str`] is by itself lenient
//! (a malformed `%X` survives as the literal characters), so the
//! validation happens in this module before the decode call.

use std::borrow::Cow;

use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};

use crate::error::NixUriError;

/// ASCII bytes that must be percent-encoded inside a query key or value.
/// The complement is RFC 3986's unreserved set (`A-Z`, `a-z`, `0-9`,
/// `-`, `.`, `_`, `~`) plus `:@/?` (the characters Nix leaves unescaped
/// inside a query). Non-ASCII bytes are always encoded by
/// `utf8_percent_encode` regardless of the set.
pub(crate) const QUERY_VALUE: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'!')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b'+')
    .add(b',')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

/// ASCII bytes that must be percent-encoded inside a fragment. Same as
/// [`QUERY_VALUE`] plus `:@/?`: Nix percent-encodes those four bytes
/// inside a fragment.
pub(crate) const FRAGMENT: &AsciiSet = &QUERY_VALUE.add(b':').add(b'@').add(b'/').add(b'?');

pub(crate) fn encode_query(s: &str) -> Cow<'_, str> {
    utf8_percent_encode(s, QUERY_VALUE).into()
}

pub(crate) fn encode_fragment(s: &str) -> Cow<'_, str> {
    utf8_percent_encode(s, FRAGMENT).into()
}

/// Re-encode a single path segment so the `/` separators inside it survive a
/// `Display` round-trip without being mistaken for the segment boundary.
///
/// Matches Nix's per-segment percent-encoding of URL paths (each segment
/// is percent-encoded individually so `/` between segments stays raw).
/// Applied to a [`super::GitForge`] owner that the parser decoded out of
/// `<scheme>:<seg>%2F<seg>/<repo>`: the validator stores the decoded
/// `seg/seg` form, and Display re-emits `%2F` between the segments so the
/// literal `/` does not collide with the owner-vs-repo boundary.
pub(crate) fn encode_path_segment(s: &str) -> Cow<'_, str> {
    if !s.contains('/') {
        return Cow::Borrowed(s);
    }
    s.replace('/', "%2F").into()
}

/// Returns [`NixUriError::InvalidUrl`] when a `%` byte is not followed by
/// exactly two ASCII hex digits, or when the decoded byte sequence is not
/// valid UTF-8. Strict where the underlying `percent_decode_str` is
/// lenient: this is what makes `?dir=%2` and `?dir=%XY` reject at parse
/// time instead of silently leaving the malformed bytes in the value.
pub(crate) fn decode_percent(s: &str) -> Result<Cow<'_, str>, NixUriError> {
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
    percent_decode_str(s)
        .decode_utf8()
        .map_err(|_| NixUriError::InvalidUrl(format!("invalid utf-8 percent-encoding in '{s}'")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_is_encoded_in_query_value() {
        assert_eq!(encode_query("foo bar"), "foo%20bar");
    }

    #[test]
    fn slash_is_kept_in_query_value() {
        assert_eq!(encode_query("foo/bar"), "foo/bar");
    }

    #[test]
    fn slash_is_encoded_in_fragment() {
        assert_eq!(encode_fragment("foo/bar"), "foo%2Fbar");
    }

    #[test]
    fn non_ascii_is_encoded() {
        assert_eq!(encode_query("fÖö"), "f%C3%96%C3%B6");
        assert_eq!(encode_fragment("fÖö"), "f%C3%96%C3%B6");
    }

    #[test]
    fn decode_round_trip() {
        let encoded = encode_query("foo bar/baz");
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
    fn decode_passes_valid_full() {
        assert_eq!(decode_percent("foo%20bar").unwrap(), "foo bar");
    }
}
