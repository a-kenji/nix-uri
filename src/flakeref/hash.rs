//! Hash decoding and rendering: `<algo>:<digest>`, SRI `<algo>-<base64>`, base16, Nix's base-32 and base64.

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

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

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
