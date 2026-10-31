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
