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

/// Returns `true` if `s` parses as a SHA-1 hash, see [`parse_hash`].
pub(crate) fn is_sha1_hash(s: &str) -> bool {
    parse_hash(s, Some(HashAlgo::Sha1)).is_some()
}

pub(crate) fn rev_to_base16(rev: &str) -> Option<String> {
    let digest = parse_hash(rev, None).or_else(|| parse_hash(rev, Some(HashAlgo::Sha1)))?;
    let mut base16 = String::with_capacity(digest.len() * 2);
    for b in digest {
        base16.push(char::from(BASE16_CHARS[usize::from(b >> 4)]));
        base16.push(char::from(BASE16_CHARS[usize::from(b & 0xf)]));
    }
    Some(base16)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HashAlgo {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

impl HashAlgo {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "md5" => Some(Self::Md5),
            "sha1" => Some(Self::Sha1),
            "sha256" => Some(Self::Sha256),
            "sha512" => Some(Self::Sha512),
            _ => None,
        }
    }

    fn size(self) -> usize {
        match self {
            Self::Md5 => 16,
            Self::Sha1 => 20,
            Self::Sha256 => 32,
            Self::Sha512 => 64,
        }
    }
}

/// Decode a hash: `<algo>:<digest>`, an SRI `<algo>-<base64>`, or a bare digest of the
/// `default` algorithm.
fn parse_hash(s: &str, default: Option<HashAlgo>) -> Option<Vec<u8>> {
    let (algo, digest, is_sri) = match s.split_once(':') {
        Some((algo, digest)) => (Some(algo), digest, false),
        None => match s.split_once('-') {
            Some((algo, digest)) => (Some(algo), digest, true),
            None => (None, s, false),
        },
    };
    let algo = match (algo.map(HashAlgo::from_name), default) {
        (Some(None), _) | (None, None) => return None,
        (Some(Some(algo)), Some(default)) if algo != default => return None,
        (Some(Some(algo)), _) | (None, Some(algo)) => algo,
    };
    let size = algo.size();
    let bytes = if is_sri || digest.len() == size.div_ceil(3) * 4 {
        decode_base64_lenient(digest)?
    } else if digest.len() == size * 2 {
        decode_base16(digest)?
    } else if digest.len() == (size * 8 - 1) / 5 + 1 {
        decode_nix32(digest)?
    } else {
        return None;
    };
    (bytes.len() == size).then_some(bytes)
}

const BASE16_CHARS: &[u8; 16] = b"0123456789abcdef";

fn decode_base16(s: &str) -> Option<Vec<u8>> {
    let (pairs, rest) = s.as_bytes().as_chunks::<2>();
    if !rest.is_empty() {
        return None;
    }
    pairs
        .iter()
        .map(|&[high, low]| Some((hex_digit(high)? << 4) | hex_digit(low)?))
        .collect()
}

fn hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

const NIX32_CHARS: &[u8] = b"0123456789abcdfghijklmnpqrsvwxyz";

/// Nix's base-32: the last character holds the lowest five bits.
fn decode_nix32(s: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    for (n, c) in s.bytes().rev().enumerate() {
        let digit = NIX32_CHARS.iter().position(|&d| d == c)?;
        let digit = u16::try_from(digit).ok()?;
        let (i, j) = (n * 5 / 8, n * 5 % 8);
        bytes.resize(i + 1, 0);
        bytes[i] |= u8::try_from((digit << j) & 0xff).ok()?;
        let carry = digit >> (8 - j);
        if carry != 0 {
            bytes.resize(i + 2, 0);
            bytes[i + 1] |= u8::try_from(carry).ok()?;
        }
    }
    Some(bytes)
}

const SHA256_BYTES: usize = 32;

const BASE64_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The padded SRI form of the SHA-256 digest a `narHash` decodes to.
pub(crate) fn render_nar_hash(nar_hash: &str) -> String {
    let digest = if nar_hash.is_empty() {
        Some(vec![0; SHA256_BYTES])
    } else {
        nar_hash
            .strip_prefix("sha256-")
            .and_then(decode_base64_lenient)
            .filter(|d| d.len() == SHA256_BYTES)
    };
    digest.map_or_else(
        || nar_hash.to_string(),
        |d| format!("sha256-{}", encode_base64(&d)),
    )
}

/// Stops at the first `=`, skips newlines, and drops the bits of an incomplete last byte.
fn decode_base64_lenient(s: &str) -> Option<Vec<u8>> {
    let mut bits: u32 = 0;
    let mut nbits = 0;
    let mut bytes = Vec::with_capacity(s.len() * 3 / 4);
    for c in s.bytes() {
        match c {
            b'=' => break,
            b'\n' => continue,
            _ => {}
        }
        let value = BASE64_CHARS.iter().position(|&d| d == c)?;
        bits = (bits << 6) | u32::try_from(value).ok()?;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            bytes.push(u8::try_from((bits >> nbits) & 0xff).ok()?);
        }
    }
    Some(bytes)
}

fn encode_base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(char::from(
                BASE64_CHARS[(n >> (18 - 6 * i) & 0x3f) as usize],
            ));
        }
        for _ in chunk.len()..3 {
            out.push('=');
        }
    }
    out
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

/// Validate a ref name with [`is_legal_ref_name`]. Returns the value owned
/// on success so the caller can hand it directly into a typed slot.
pub(crate) fn validated_ref_name(value: &str) -> Result<String, NixUriError> {
    if !is_legal_ref_name(value) {
        return Err(NixUriError::InvalidValue {
            field: "ref",
            reason: "expected a Git branch or tag name (see `git check-ref-format`)".to_string(),
        });
    }
    Ok(value.to_string())
}

/// Returns `true` if `s` is a valid git-forge `?host=` value.
pub(crate) fn is_valid_host_name(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
}

/// Validate a `?host=` candidate. Returns the value owned on success.
pub(crate) fn validated_host_name(value: &str) -> Result<String, NixUriError> {
    if !is_valid_host_name(value) {
        return Err(NixUriError::InvalidValue {
            field: "host",
            reason: "expected a forge host (alphanumeric, '.', '-')".to_string(),
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
    #[case(
        "sha1-AAAAAAAAAAAAAAAAAAAAAAAAAAA=x",
        "0000000000000000000000000000000000000000"
    )]
    #[case("md5-AAAAAAAAAAAAAAAAAAAAAA==", "00000000000000000000000000000000")]
    #[case(
        "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "0000000000000000000000000000000000000000000000000000000000000000"
    )]
    fn rev_to_base16(#[case] input: &str, #[case] base16: &str) {
        assert_eq!(super::rev_to_base16(input).as_deref(), Some(base16));
    }

    #[test]
    fn nix32_round_trips_a_known_hash() {
        // `nix hash convert --hash-algo sha1 --to nix32
        // a3a3dda3bacf61e8a39258a0ed9c924eeca8e293`
        assert_eq!(
            super::rev_to_base16("jgiaiv2fjaffv82qjaiyhqfgpaixv8x3").as_deref(),
            Some("a3a3dda3bacf61e8a39258a0ed9c924eeca8e293")
        );
    }

    #[test]
    fn rev_to_base16_rejects_what_nix_cannot_decode() {
        assert_eq!(super::rev_to_base16("abc"), None);
        assert_eq!(super::rev_to_base16(&"0".repeat(64)), None);
        assert_eq!(
            super::rev_to_base16("sha1-AAAAAAAAAAAAAAAAAAAAAAAAAAAB="),
            None
        );
        assert_eq!(super::rev_to_base16("foo-abc"), None);
    }

    #[test]
    fn forge_rev_must_be_sha1() {
        assert!(is_sha1_hash("sha1-AAAAAAAAAAAAAAAAAAAAAAAAAAA=x"));
        assert!(!is_sha1_hash(
            "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
        ));
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

    const H0: &str = "sha256-qQ2Zz4DNHUpgVCi6rTS7EE4+RMqFQtUfWF26NUtJKS0=";

    #[rstest]
    #[case::canonical(H0, H0)]
    #[case::trailing_bits("sha256-qQ2Zz4DNHUpgVCi6rTS7EE4+RMqFQtUfWF26NUtJKS1=", H0)]
    #[case::unpadded("sha256-qQ2Zz4DNHUpgVCi6rTS7EE4+RMqFQtUfWF26NUtJKS0", H0)]
    #[case::after_padding("sha256-qQ2Zz4DNHUpgVCi6rTS7EE4+RMqFQtUfWF26NUtJKS0=!?", H0)]
    #[case::newline("sha256-qQ2Zz4DNHUpgVCi6rTS7EE4+\nRMqFQtUfWF26NUtJKS0=", H0)]
    #[case::empty("", "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")]
    fn nar_hash_renders_canonically(#[case] nar_hash: &str, #[case] rendered: &str) {
        assert_eq!(render_nar_hash(nar_hash), rendered);
    }

    #[rstest]
    #[case::short("sha256-hH")]
    #[case::bad_char("sha256-qQ2Zz4DNHUpgVCi6rTS7EE4*RMqFQtUfWF26NUtJKS0=")]
    #[case::sha512("sha512-qQ2Zz4DNHUpgVCi6rTS7EE4+RMqFQtUfWF26NUtJKS0=")]
    #[case::not_sri("qQ2Zz4DNHUpgVCi6rTS7EE4+RMqFQtUfWF26NUtJKS0=")]
    fn undecodable_nar_hash_renders_as_written(#[case] nar_hash: &str) {
        assert_eq!(render_nar_hash(nar_hash), nar_hash);
    }
}
