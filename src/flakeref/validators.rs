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

/// Returns `true` if `s` matches a revision shape: exactly 40 ASCII hex
/// digits (SHA-1) or exactly 64 ASCII hex digits (SHA-256),
/// case-insensitive.
///
/// Used by the parser to split a single path-component value into the
/// typed `rev` slot or the typed `ref_` slot.
pub(crate) fn looks_like_rev(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Returns `true` if `s` is a 40-character hex SHA-1 commit hash.
pub(crate) fn is_sha1_hex(s: &str) -> bool {
    s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Returns `true` if `s` parses as a SHA-1 hash in any accepted encoding
/// for a git-forge `?rev=`: base16 (40 chars), nix32 (32 chars)
/// or padded base64 (28 chars), optionally prefixed with `sha1:`, or an
/// SRI `sha1-<base64>` hash.
pub(crate) fn is_sha1_hash(s: &str) -> bool {
    if let Some(base64) = s.strip_prefix("sha1-") {
        return is_sha1_base64(base64);
    }
    let digest = s.strip_prefix("sha1:").unwrap_or(s);
    match digest.len() {
        40 => digest.chars().all(|c| c.is_ascii_hexdigit()),
        32 => digest.chars().all(|c| NIX32_CHARS.contains(c)),
        28 => is_sha1_base64(digest),
        _ => false,
    }
}

const NIX32_CHARS: &str = "0123456789abcdfghijklmnpqrsvwxyz";

/// 20 bytes of base64 are 27 significant characters and one `=` of
/// padding.
fn is_sha1_base64(s: &str) -> bool {
    s.len() == 28
        && s.ends_with('=')
        && s[..27]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/')
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

/// Returns `true` if `s` is a legal Git branch or tag name, the check
/// applied to every git, git-forge and indirect ref. It follows
/// `git check-ref-format`: no `..`, `@{`, control characters, space or
/// any of `~^:?*[\`, no empty component, no component starting with `.`
/// or ending in `.lock`, and the whole name must not be `@`, start with
/// `-` or end with `.`.
pub(crate) fn is_legal_ref_name(s: &str) -> bool {
    if s.is_empty()
        || s == "@"
        || s.starts_with('-')
        || s.ends_with('.')
        || s.contains("..")
        || s.contains("@{")
    {
        return false;
    }
    if s.chars().any(|c| {
        c.is_ascii_control() || matches!(c, ' ' | '~' | '^' | ':' | '?' | '*' | '[' | '\\')
    }) {
        return false;
    }
    s.split('/').all(|component| {
        !component.is_empty() && !component.starts_with('.') && !component.ends_with(".lock")
    })
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

/// Returns `true` if `s` is a valid git-forge `?host=` value: ASCII alphanumerics, `.`, and `-`. The empty string is
/// accepted (semantically equivalent to no override).
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

    #[test]
    fn lowercase_40_hex_is_rev() {
        assert!(looks_like_rev("b2df4e4e80e04cbb33a350f87717f4bd6140d298"));
    }

    #[test]
    fn uppercase_40_hex_is_rev() {
        assert!(looks_like_rev("B2DF4E4E80E04CBB33A350F87717F4BD6140D298"));
    }

    #[test]
    fn mixed_case_40_hex_is_rev() {
        assert!(looks_like_rev("B2df4E4e80E04cBb33A350f87717F4bd6140D298"));
    }

    #[test]
    fn thirty_nine_hex_is_not_rev() {
        assert!(!looks_like_rev("b2df4e4e80e04cbb33a350f87717f4bd6140d29"));
    }

    #[test]
    fn forty_one_hex_is_not_rev() {
        assert!(!looks_like_rev("b2df4e4e80e04cbb33a350f87717f4bd6140d2980"));
    }

    #[test]
    fn sixty_four_hex_is_rev() {
        // SHA-256.
        assert!(looks_like_rev(
            "0000000000000000000000000000000000000000000000000000000000000000"
        ));
    }

    #[test]
    fn sixty_four_hex_uppercase_is_rev() {
        assert!(looks_like_rev(
            "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789"
        ));
    }

    #[test]
    fn sixty_three_hex_is_not_rev() {
        assert!(!looks_like_rev(
            "000000000000000000000000000000000000000000000000000000000000000"
        ));
    }

    #[test]
    fn sixty_five_hex_is_not_rev() {
        assert!(!looks_like_rev(
            "00000000000000000000000000000000000000000000000000000000000000000"
        ));
    }

    #[test]
    fn fifty_hex_is_not_rev() {
        assert!(!looks_like_rev(
            "00000000000000000000000000000000000000000000000000"
        ));
    }

    #[test]
    fn forty_chars_with_non_hex_is_not_rev() {
        // `g` is non-hex; same length but not a rev.
        assert!(!looks_like_rev("g2df4e4e80e04cbb33a350f87717f4bd6140d298"));
    }

    #[test]
    fn release_branch_name_is_not_rev() {
        assert!(!looks_like_rev("release-23.05"));
    }

    #[test]
    fn empty_is_not_rev() {
        assert!(!looks_like_rev(""));
    }

    #[test]
    fn short_alpha_is_not_rev() {
        assert!(!looks_like_rev("main"));
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
        // Reserved in URL grammar; would terminate the path-component slot anyway.
        assert!(!matches_ref_shape("br?anch"));
    }

    #[test]
    fn query_bool_is_true_only_for_one() {
        assert!(query_bool("1"));
        for value in ["0", "true", "false", "yes", ""] {
            assert!(!query_bool(value), "{value:?}");
        }
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
