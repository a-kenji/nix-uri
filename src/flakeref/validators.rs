//! Small classifiers for ref/rev strings and query values.
//!
//! Path-component slots like `github:owner/repo/<x>` carry either a Git ref
//! name or a commit hash, told apart by the hex shape.

use crate::error::NixUriError;

/// Coerce a boolean query parameter (`submodules`, `shallow`,
/// `lfs`, ...): `"1"` is true and every other value, `"true"` included,
/// is false.
pub(crate) fn query_bool(value: &str) -> bool {
    value == "1"
}

/// Returns `true` if `s` is a 40-character hex SHA-1 commit hash.
pub(crate) fn is_sha1_hex(s: &str) -> bool {
    s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Returns `true` if `s` parses as a SHA-1 hash in any encoding Nix accepts for a
/// revision, see [`sha1_to_base16`].
pub(crate) fn is_sha1_hash(s: &str) -> bool {
    sha1_to_base16(s).is_some()
}

const SHA1_BYTES: usize = 20;

/// Decode a SHA-1 hash given as base16 (40 chars), nix32 (32 chars) or padded base64 (28
/// chars), optionally prefixed with `sha1:`, or as an SRI `sha1-<base64>` hash, and
/// render it as lower-case base16.
pub(crate) fn sha1_to_base16(s: &str) -> Option<String> {
    let bytes = if let Some(base64) = s.strip_prefix("sha1-") {
        decode_base64(base64)?
    } else {
        let digest = s.strip_prefix("sha1:").unwrap_or(s);
        match digest.len() {
            40 => decode_base16(digest)?,
            32 => decode_nix32(digest)?,
            28 => decode_base64(digest)?,
            _ => return None,
        }
    };
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn decode_base16(s: &str) -> Option<Vec<u8>> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

const NIX32_CHARS: &[u8] = b"0123456789abcdfghijklmnpqrsvwxyz";

/// Nix's base-32: the last character holds the lowest five bits.
fn decode_nix32(s: &str) -> Option<Vec<u8>> {
    let mut bytes = vec![0u8; SHA1_BYTES];
    for (n, c) in s.bytes().rev().enumerate() {
        let digit = NIX32_CHARS.iter().position(|&d| d == c)?;
        let digit = u8::try_from(digit).ok()?;
        let bit = n * 5;
        let (i, j) = (bit / 8, bit % 8);
        bytes[i] |= digit << j;
        let carry = u16::from(digit) >> (8 - j);
        if i + 1 < SHA1_BYTES {
            bytes[i + 1] |= u8::try_from(carry).ok()?;
        } else if carry != 0 {
            return None;
        }
    }
    Some(bytes)
}

/// Padded base64: 20 bytes are 27 significant characters and one `=`.
fn decode_base64(s: &str) -> Option<Vec<u8>> {
    let digits = s.strip_suffix('=')?;
    if digits.len() != 27 {
        return None;
    }
    let mut bits: u32 = 0;
    let mut nbits = 0;
    let mut bytes = Vec::with_capacity(SHA1_BYTES);
    for c in digits.bytes() {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        bits = (bits << 6) | u32::from(value);
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            bytes.push(u8::try_from((bits >> nbits) & 0xff).ok()?);
        }
    }
    (bytes.len() == SHA1_BYTES).then_some(bytes)
}

/// Returns `true` if `s` matches the ref shape of the flake-id shorthand
/// (`nixpkgs/<ref>`) and of Mercurial refs: a non-empty string whose first
/// character is alphanumeric or `@`, and whose remainder is drawn from
/// `[a-zA-Z0-9_.\/@+-]`.
pub(crate) fn matches_ref_shape(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphanumeric() || first == '@') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '@' | '+' | '-'))
}

/// Returns `true` if `s` is a legal Git ref name, the check applied to every git,
/// git-forge and indirect ref. This takes any name libgit2 accepts as a reference, branch
/// or tag name, apart from `@` and names containing DEL:
///
/// - Every `/`-separated component is non-empty, does not start with `.` or end in
///   `.lock`, and contains no `..`, `@{`, control character, space or any of `~^:?*[\`,
///   and the name does not end with `.`.
/// - As a branch or tag name it must not start with `-` or be `HEAD`.
/// - As a full reference name its first component may start with `^`, a one-level name
///   must be [all caps](is_all_caps_name) (`HEAD`, `FETCH_HEAD`) and a multi-level name
///   must not start with an all-caps component, so `-a/b` is legal where `-a` is not.
pub(crate) fn is_legal_ref_name(s: &str) -> bool {
    if s == "@" || s.contains('\u{7f}') {
        return false;
    }
    let is_branch_name = !s.starts_with('-') && s != "HEAD" && has_valid_components(s, false);
    let is_reference_name = has_valid_components(s, true)
        && match s.split_once('/') {
            None => is_all_caps_name(s),
            Some((first, _)) => !is_all_caps_name(first),
        };
    is_branch_name || is_reference_name
}

fn has_valid_components(s: &str, caret_prefix: bool) -> bool {
    !s.ends_with('.')
        && s.split('/').enumerate().all(|(i, component)| {
            let body = match component.strip_prefix('^') {
                Some(body) if caret_prefix && i == 0 => body,
                _ => component,
            };
            !component.is_empty()
                && !component.starts_with('.')
                && !component.ends_with(".lock")
                && !body.contains("..")
                && !body.contains("@{")
                && !body.chars().any(|c| {
                    c.is_ascii_control()
                        || matches!(c, ' ' | '~' | '^' | ':' | '?' | '*' | '[' | '\\')
                })
        })
}

fn is_all_caps_name(s: &str) -> bool {
    let body = s.strip_prefix('^').unwrap_or(s);
    s == "@"
        || (body.chars().all(|c| c.is_ascii_uppercase() || c == '_')
            && !s.starts_with('_')
            && !s.ends_with('_'))
}

/// Diagnostic text shared by every parser site that funnels a ref-name
/// rejection through [`NixUriError::InvalidValue`].
pub(crate) const REF_NAME_REJECTION: &str =
    "expected a Git branch or tag name (see `git check-ref-format`)";

/// Validate a ref name with [`is_legal_ref_name`]. Returns the value owned
/// on success so the caller can hand it directly into a typed slot.
pub(crate) fn validated_ref_name(value: &str) -> Result<String, NixUriError> {
    if !is_legal_ref_name(value) {
        return Err(NixUriError::InvalidValue {
            field: "ref",
            reason: REF_NAME_REJECTION.to_string(),
        });
    }
    Ok(value.to_string())
}

/// Returns `true` if `s` is a valid git-forge `?host=` value.
pub(crate) fn validate_host_name(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
}

/// Diagnostic text shared by the routing site that funnels a `?host=`
/// rejection through [`NixUriError::InvalidValue`].
pub(crate) const HOST_NAME_REJECTION: &str = "expected a forge host (alphanumeric, '.', '-')";

/// Validate a `?host=` candidate. Returns the value owned on success.
pub(crate) fn validated_host_name(value: &str) -> Result<String, NixUriError> {
    if !validate_host_name(value) {
        return Err(NixUriError::InvalidValue {
            field: "host",
            reason: HOST_NAME_REJECTION.to_string(),
        });
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("b2df4e4e80e04cbb33a350f87717f4bd6140d298")]
    #[case("B2DF4E4E80E04CBB33A350F87717F4BD6140D298")]
    fn sha1_hex(#[case] s: &str) {
        assert!(is_sha1_hex(s));
    }

    /// Only 40 hex digits are a revision in a path segment.
    #[rstest]
    #[case("b2df4e4e80e04cbb33a350f87717f4bd6140d29")]
    #[case("b2df4e4e80e04cbb33a350f87717f4bd6140d2980")]
    #[case("0000000000000000000000000000000000000000000000000000000000000000")]
    #[case("g2df4e4e80e04cbb33a350f87717f4bd6140d298")]
    #[case("main")]
    #[case("")]
    fn not_sha1_hex(#[case] s: &str) {
        assert!(!is_sha1_hex(s));
    }

    /// `flakeRefToString` renders a revision as lower-case base16.
    #[rstest]
    #[case(
        "A3A3DDA3BACF61E8A39258A0ED9C924EECA8E293",
        "a3a3dda3bacf61e8a39258a0ed9c924eeca8e293"
    )]
    #[case(
        "sha1-AAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        "0000000000000000000000000000000000000000"
    )]
    #[case(
        "00000000000000000000000000000000",
        "0000000000000000000000000000000000000000"
    )]
    #[case(
        "sha1:0000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000"
    )]
    fn sha1_to_base16(#[case] input: &str, #[case] base16: &str) {
        assert_eq!(super::sha1_to_base16(input).as_deref(), Some(base16));
    }

    #[test]
    fn nix32_round_trips_a_known_hash() {
        // `nix hash convert --hash-algo sha1 --to nix32
        // a3a3dda3bacf61e8a39258a0ed9c924eeca8e293`
        assert_eq!(
            super::sha1_to_base16("jgiaiv2fjaffv82qjaiyhqfgpaixv8x3").as_deref(),
            Some("a3a3dda3bacf61e8a39258a0ed9c924eeca8e293")
        );
    }

    #[test]
    fn sha1_to_base16_rejects_other_lengths() {
        assert_eq!(super::sha1_to_base16("abc"), None);
        assert_eq!(super::sha1_to_base16(&"0".repeat(64)), None);
    }

    #[test]
    fn validate_ref_accepts_simple_branch() {
        assert!(matches_ref_shape("main"));
    }

    #[test]
    fn validate_ref_accepts_release_branch() {
        assert!(matches_ref_shape("release-23.05"));
    }

    #[test]
    fn validate_ref_accepts_namespaced_branch() {
        assert!(matches_ref_shape("feature/foo-bar"));
    }

    #[test]
    fn validate_ref_accepts_version_tag() {
        assert!(matches_ref_shape("v1.2.3"));
    }

    #[test]
    fn validate_ref_accepts_at_prefixed() {
        assert!(matches_ref_shape("@HEAD"));
    }

    #[test]
    fn validate_ref_rejects_empty() {
        assert!(!matches_ref_shape(""));
    }

    #[test]
    fn validate_ref_rejects_leading_dash() {
        assert!(!matches_ref_shape("-leading-dash"));
    }

    #[test]
    fn validate_ref_rejects_leading_dot() {
        assert!(!matches_ref_shape(".hidden"));
    }

    #[test]
    fn validate_ref_rejects_leading_slash() {
        assert!(!matches_ref_shape("/abs/branch"));
    }

    #[test]
    fn validate_ref_rejects_whitespace() {
        assert!(!matches_ref_shape("invalid ref"));
        assert!(!matches_ref_shape("trailing\t"));
    }

    #[test]
    fn validate_ref_rejects_question_mark() {
        // Reserved in URL grammar, it would terminate the path-component slot anyway.
        assert!(!matches_ref_shape("br?anch"));
    }

    #[rstest::rstest]
    #[case("1", true)]
    #[case("0", false)]
    #[case("true", false)]
    #[case("false", false)]
    #[case("yes", false)]
    #[case("", false)]
    fn query_bool_is_true_only_for_one(#[case] value: &str, #[case] expected: bool) {
        assert_eq!(query_bool(value), expected);
    }

    #[rstest::rstest]
    #[case("main")]
    #[case("release-23.05")]
    #[case("feature/foo-bar")]
    #[case("_under")]
    #[case("x,y")]
    #[case("a!b")]
    #[case("a#b")]
    #[case("@foo")]
    #[case("HEAD")]
    #[case("refs/heads/-x")]
    #[case("a/-b")]
    #[case("a./b")]
    #[case("x.lock2")]
    fn legal_ref_names(#[case] name: &str) {
        assert!(is_legal_ref_name(name), "{name:?}");
    }

    #[rstest::rstest]
    #[case("")]
    #[case("@")]
    #[case("a..b")]
    #[case(".hidden")]
    #[case("a/.b")]
    #[case("-dash")]
    #[case("a.lock")]
    #[case("a/b.lock/c")]
    #[case("end.")]
    #[case("end/")]
    #[case("/lead")]
    #[case("a//b")]
    #[case("a@{b")]
    #[case("a b")]
    #[case("a~b")]
    #[case("a^b")]
    #[case("a:b")]
    #[case("a?b")]
    #[case("a*b")]
    #[case("a[b")]
    #[case("a\\b")]
    #[case("a\u{1}b")]
    #[case("a\u{7f}b")]
    fn illegal_ref_names(#[case] name: &str) {
        assert!(!is_legal_ref_name(name), "{name:?}");
    }

    #[rstest::rstest]
    #[case("0000000000000000000000000000000000000000")]
    #[case("sha1:0000000000000000000000000000000000000000")]
    #[case("00000000000000000000000000000000")]
    #[case("AAAAAAAAAAAAAAAAAAAAAAAAAAA=")]
    #[case("AAAAAAAAAAAAAAAAAAAAAAAAAAB=")]
    #[case("sha1:AAAAAAAAAAAAAAAAAAAAAAAAAAA=")]
    #[case("sha1-AAAAAAAAAAAAAAAAAAAAAAAAAAA=")]
    fn sha1_hashes(#[case] hash: &str) {
        assert!(is_sha1_hash(hash), "{hash:?}");
    }

    #[rstest::rstest]
    #[case("abc")]
    #[case("0000000000000000000000000000000000000000000000000000000000000000")]
    #[case("AAAAAAAAAAAAAAAAAAAAAAAAAAAA")]
    #[case("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee")]
    #[case("sha1-0000000000000000000000000000000000000000")]
    #[case("SHA1:0000000000000000000000000000000000000000")]
    #[case("sha256:0000000000000000000000000000000000000000")]
    fn not_sha1_hashes(#[case] hash: &str) {
        assert!(!is_sha1_hash(hash), "{hash:?}");
    }

    #[test]
    fn validated_ref_name_returns_invalid_value_error() {
        let err = validated_ref_name("-bad").expect_err("must reject");
        match err {
            NixUriError::InvalidValue { field, reason } => {
                assert_eq!(field, "ref");
                assert!(reason.contains("check-ref-format"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
