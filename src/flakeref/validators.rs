//! Small classifiers for ref/rev strings.
//!
//! Path-component slots like `github:owner/repo/<x>` carry either a Git ref
//! name or a 40-character commit hash. Nix discriminates on the 40-hex
//! shape; everything else is treated as a ref name.

use crate::error::NixUriError;

/// Parse a boolean query-parameter value. Nix's URL-time coercion is
/// strictly `value == "1"`: `"1"` -> `true`, `"0"` -> `false`. Anything
/// else (including `"true"` / `"false"`, which look right but Nix maps to
/// `false`) returns [`NixUriError::InvalidValue`] tagged with the field
/// name so a downstream caller can pinpoint the offending key.
///
/// Rejecting `"true"` / `"false"` outright keeps the diagnostic visible:
/// silently coercing them to `false` would match Nix's runtime behaviour
/// but lose the parse-time signal that the caller wrote the wrong wire
/// form.
pub(crate) fn parse_bool_param(field: &'static str, value: &str) -> Result<bool, NixUriError> {
    match value {
        "1" => Ok(true),
        "0" => Ok(false),
        _ => Err(NixUriError::InvalidValue {
            field,
            reason:
                "expected \"1\" or \"0\" (upstream URL-time coercion is strictly value == \"1\")"
                    .to_string(),
        }),
    }
}

/// Returns `true` if `s` matches a Nix-recognised revision shape:
/// exactly 40 ASCII hex digits (SHA-1) or exactly 64 ASCII hex digits
/// (SHA-256), case-insensitive. Both algorithms are supported for git
/// inputs.
///
/// Used by the parser to split a single path-component value into the
/// typed `rev` slot or the typed `ref_` slot, and by
/// [`crate::parser::apply_param_ref_rev`] to gate `?rev=` query values.
pub(crate) fn looks_like_rev(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Returns `true` if `s` matches Nix's accepted ref-name shape: a non-empty
/// string whose first character is alphanumeric or `@`, and whose remainder
/// is drawn from `[a-zA-Z0-9_.\/@+-]`.
///
/// Matches Nix byte-for-byte so a value that nix-uri accepts as a ref is
/// one a downstream Nix fetcher would also accept; values that fail here
/// surface as [`NixUriError::InvalidValue`] at parse time, keeping the
/// failure off the consumer's fetch path.
pub(crate) fn validate_ref_name(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphanumeric() || first == '@') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '@' | '+' | '-'))
}

/// Diagnostic text shared by every parser site that funnels a ref-name
/// rejection through [`NixUriError::InvalidValue`]. Centralised so the
/// error reads consistently regardless of which routing path produced it.
pub(crate) const REF_NAME_REJECTION: &str = "expected a Git ref name (alphanumeric, '_', '.', '/', '@', '+', '-'; \
     must start with an alphanumeric or '@')";

/// Validate a ref-name candidate at a routing site that surfaces failures
/// as [`NixUriError::InvalidValue`]. Returns the value owned on success so
/// the caller can hand it directly into a typed slot.
pub(crate) fn validated_ref_name(value: &str) -> Result<String, NixUriError> {
    if !validate_ref_name(value) {
        return Err(NixUriError::InvalidValue {
            field: "ref",
            reason: REF_NAME_REJECTION.to_string(),
        });
    }
    Ok(value.to_string())
}

/// Returns `true` if `s` matches Nix's accepted `?host=` value shape:
/// ASCII alphanumerics, `.`, and `-`. The empty string is accepted
/// (semantically equivalent to no override; the fetch layer handles the
/// empty case).
pub(crate) fn validate_host_name(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
}

/// Diagnostic text shared by the routing site that funnels a `?host=`
/// rejection through [`NixUriError::InvalidValue`].
pub(crate) const HOST_NAME_REJECTION: &str = "expected a forge host (alphanumeric, '.', '-')";

/// Validate a `?host=` candidate at the routing site. Returns the value
/// owned on success so the caller can hand it directly into the typed
/// slot.
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
        assert!(validate_ref_name("main"));
    }

    #[test]
    fn validate_ref_accepts_release_branch() {
        assert!(validate_ref_name("release-23.05"));
    }

    #[test]
    fn validate_ref_accepts_namespaced_branch() {
        assert!(validate_ref_name("feature/foo-bar"));
    }

    #[test]
    fn validate_ref_accepts_version_tag() {
        assert!(validate_ref_name("v1.2.3"));
    }

    #[test]
    fn validate_ref_accepts_at_prefixed() {
        assert!(validate_ref_name("@HEAD"));
    }

    #[test]
    fn validate_ref_rejects_empty() {
        assert!(!validate_ref_name(""));
    }

    #[test]
    fn validate_ref_rejects_leading_dash() {
        assert!(!validate_ref_name("-leading-dash"));
    }

    #[test]
    fn validate_ref_rejects_leading_dot() {
        assert!(!validate_ref_name(".hidden"));
    }

    #[test]
    fn validate_ref_rejects_leading_slash() {
        assert!(!validate_ref_name("/abs/branch"));
    }

    #[test]
    fn validate_ref_rejects_whitespace() {
        assert!(!validate_ref_name("invalid ref"));
        assert!(!validate_ref_name("trailing\t"));
    }

    #[test]
    fn validate_ref_rejects_question_mark() {
        // Reserved in URL grammar; would terminate the path-component slot anyway.
        assert!(!validate_ref_name("br?anch"));
    }

    #[test]
    fn parse_bool_param_accepts_one_as_true() {
        assert!(parse_bool_param("lfs", "1").unwrap());
    }

    #[test]
    fn parse_bool_param_accepts_zero_as_false() {
        assert!(!parse_bool_param("lfs", "0").unwrap());
    }

    #[test]
    fn parse_bool_param_rejects_true_string() {
        // Nix's URL-time coercion is strictly `value == "1"`, so `"true"`
        // is *not* an alias for true. Surface the divergence as
        // InvalidValue rather than silently mapping it to false (which
        // would lose the diagnostic).
        let err = parse_bool_param("lfs", "true").expect_err("must reject");
        match err {
            NixUriError::InvalidValue { field, .. } => assert_eq!(field, "lfs"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn parse_bool_param_rejects_false_string() {
        let err = parse_bool_param("lfs", "false").expect_err("must reject");
        assert!(matches!(
            err,
            NixUriError::InvalidValue { field: "lfs", .. }
        ));
    }

    #[test]
    fn parse_bool_param_rejects_garbage() {
        let err = parse_bool_param("lfs", "yes").expect_err("must reject");
        assert!(matches!(
            err,
            NixUriError::InvalidValue { field: "lfs", .. }
        ));
    }

    #[test]
    fn validated_ref_name_returns_invalid_value_error() {
        let err = validated_ref_name("-bad").expect_err("must reject");
        match err {
            NixUriError::InvalidValue { field, reason } => {
                assert_eq!(field, "ref");
                assert!(reason.contains("Git ref name"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
