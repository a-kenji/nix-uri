//! A URL, parsed and rendered.
//!
//! URLs are parsed against RFC 3986, keeping the path as a list of decoded segments and
//! the query as a map from decoded key to decoded value (sorted, the first of a repeated
//! key wins), and rendered back percent-encoded.

use std::collections::BTreeMap;
use std::fmt::{self, Display};

use crate::{
    error::{NixUriError, NixUriResult, UnsupportedReason},
    flakeref::encoding::{
        check_percent_escapes, decode_percent, encode_ip_literal, encode_path_segment,
        encode_query, encode_unreserved, is_query_char, is_reg_name_char, is_sub_delim,
        is_unreserved, is_url_path_char, is_userinfo_char,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Url {
    pub(crate) scheme: String,
    pub(crate) authority: Option<Authority>,
    /// Decoded path segments. An absolute path starts with an empty segment, so `/a/b` is
    /// `["", "a", "b"]`.
    pub(crate) path: Vec<String>,
    pub(crate) query: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Authority {
    pub(crate) user: Option<String>,
    pub(crate) password: Option<String>,
    pub(crate) host: Host,
    pub(crate) port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Host {
    /// A registered name or IPv4 address, decoded.
    Name(String),
    /// The text between the brackets of an IP literal.
    Literal(String),
}

/// The application and transport of a scheme: `git+https` is `git` over `https`, `https`
/// is plain `https`.
pub(crate) fn split_scheme(scheme: &str) -> (Option<&str>, &str) {
    match scheme.split_once('+') {
        Some((application, transport)) => (Some(application), transport),
        None => (None, scheme),
    }
}

/// The scheme of `input`, if it starts with one: a letter followed by letters, digits,
/// `+`, `-` or `.`, then `:`.
pub(crate) fn scheme_of(input: &str) -> Option<&str> {
    let (scheme, _) = input.split_once(':')?;
    let mut chars = scheme.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    valid.then_some(scheme)
}

/// Split a query into its decoded `key=value` pairs. A pair without `=` is ignored, and
/// of a repeated key the first one wins. Unescaped spaces and `"` are tolerated. The
/// escapes of the whole query must be well-formed, including those of ignored pairs.
pub(crate) fn parse_query(query: &str) -> NixUriResult<BTreeMap<String, String>> {
    if !query.chars().all(is_query_char) {
        return Err(NixUriError::InvalidUrl(query.into()));
    }
    check_percent_escapes(query)?;
    let mut pairs = BTreeMap::new();
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let key = decode_percent(key)?.into_owned();
        let value = decode_percent(value)?.into_owned();
        pairs.entry(key).or_insert(value);
    }
    Ok(pairs)
}

/// `?` and the encoded `key=value` pairs, or nothing when the query is empty.
pub(crate) struct EncodedQuery<'q, K, V>(pub(crate) &'q BTreeMap<K, V>);

impl<K: AsRef<str>, V: AsRef<str>> Display for EncodedQuery<'_, K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, (key, value)) in self.0.iter().enumerate() {
            let separator = if i == 0 { '?' } else { '&' };
            write!(
                f,
                "{separator}{}={}",
                encode_query(key.as_ref()),
                encode_query(value.as_ref())
            )?;
        }
        Ok(())
    }
}

impl Url {
    pub(crate) fn parse(input: &str) -> NixUriResult<Self> {
        let invalid = || NixUriError::InvalidUrl(input.into());
        let scheme = scheme_of(input).ok_or_else(invalid)?;
        let rest = &input[scheme.len() + 1..];
        let (rest, query) = match rest.split_once('?') {
            Some((rest, query)) => (rest, parse_query(query)?),
            None => (rest, BTreeMap::new()),
        };
        let (_, transport) = split_scheme(scheme);
        let (authority, mut encoded_path) = match rest.strip_prefix("//") {
            Some(after) => {
                let end = after.find('/').unwrap_or(after.len());
                let authority = Authority::parse(&after[..end], transport)?;
                if transport == "file" && authority.host != Host::Name(String::new()) {
                    return Err(invalid());
                }
                (Some(authority), &after[end..])
            }
            None => (None, rest),
        };
        if !encoded_path.chars().all(is_url_path_char) {
            return Err(invalid());
        }
        if transport == "file" && encoded_path.is_empty() {
            encoded_path = "/";
        }
        let path = encoded_path
            .split('/')
            .map(|segment| decode_percent(segment).map(std::borrow::Cow::into_owned))
            .collect::<NixUriResult<Vec<_>>>()?;
        Ok(Self {
            scheme: scheme.to_string(),
            authority,
            path,
            query,
        })
    }

    /// Drop the application from the scheme: `git+https` becomes `https`.
    pub(crate) fn strip_application(&mut self) {
        let (_, transport) = split_scheme(&self.scheme);
        self.scheme = transport.to_string();
    }

    pub(crate) fn non_empty_segments(&self) -> impl Iterator<Item = &str> {
        self.path
            .iter()
            .map(String::as_str)
            .filter(|s| !s.is_empty())
    }
}

impl Authority {
    fn parse(input: &str, transport: &str) -> NixUriResult<Self> {
        let invalid = || NixUriError::InvalidUrl(input.into());
        let (userinfo, host_port) = match input.split_once('@') {
            Some((userinfo, host_port)) => (Some(userinfo), host_port),
            None => (None, input),
        };
        let (user, password) = match userinfo {
            Some(userinfo) => {
                if !userinfo.chars().all(is_userinfo_char) {
                    return Err(invalid());
                }
                let (user, password) = match userinfo.split_once(':') {
                    Some((user, password)) => (user, Some(password)),
                    None => (userinfo, None),
                };
                let password = password
                    .map(|p| decode_percent(p).map(std::borrow::Cow::into_owned))
                    .transpose()?;
                (Some(decode_percent(user)?.into_owned()), password)
            }
            None => (None, None),
        };
        let (host, port) = if let Some(literal) = host_port.strip_prefix('[') {
            let (literal, after) = literal.split_once(']').ok_or_else(invalid)?;
            let literal = parse_ip_literal(literal)?.ok_or_else(invalid)?;
            let port = match after {
                "" => None,
                _ => Some(after.strip_prefix(':').ok_or_else(invalid)?),
            };
            (Host::Literal(literal), port)
        } else {
            let (host, port) = match host_port.split_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (host_port, None),
            };
            if !host.chars().all(is_reg_name_char) {
                return Err(invalid());
            }
            (Host::Name(decode_percent(host)?.into_owned()), port)
        };
        let port = match port {
            None | Some("") => None,
            Some(port) if port.chars().all(|c| c.is_ascii_digit()) => match port.parse::<u16>() {
                Ok(n) if n > 0 => Some(n),
                _ => return Err(invalid()),
            },
            // An SCP-style location (`git@host:owner/repo`) written after `ssh://` reads
            // its path as the port.
            Some(_) if transport == "ssh" => {
                return Err(NixUriError::Unsupported(UnsupportedReason::ScpStyleUrl {
                    url: input.into(),
                }));
            }
            Some(_) => return Err(invalid()),
        };
        Ok(Self {
            user,
            password,
            host,
            port,
        })
    }

    pub(crate) fn host_str(&self) -> String {
        self.host.to_string()
    }
}

impl Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(name) => write!(f, "{}", encode_unreserved(name)),
            Self::Literal(literal) => write!(f, "[{}]", encode_ip_literal(literal)),
        }
    }
}

impl Display for Authority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(user) = &self.user {
            write!(f, "{}", encode_unreserved(user))?;
            if let Some(password) = &self.password {
                write!(f, ":{}", encode_unreserved(password))?;
            }
            write!(f, "@")?;
        }
        write!(f, "{}", self.host)?;
        if let Some(port) = self.port {
            write!(f, ":{port}")?;
        }
        Ok(())
    }
}

impl Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:", self.scheme)?;
        if let Some(authority) = &self.authority {
            write!(f, "//{authority}")?;
        }
        write!(
            f,
            "{}{}",
            EncodedPath(&self.path),
            EncodedQuery(&self.query)
        )
    }
}

/// The text between an IP literal's brackets: IPv6, optionally with a zone ID, or
/// `IPvFuture` (`v1.x`, lower-case `v`). The zone ID is decoded, so `fe80::1%25eth0` is
/// `fe80::1%eth0`. `None` when it is neither.
fn parse_ip_literal(literal: &str) -> NixUriResult<Option<String>> {
    if let Some(future) = literal.strip_prefix('v') {
        let valid = future.split_once('.').is_some_and(|(version, rest)| {
            !version.is_empty()
                && version.chars().all(|c| c.is_ascii_hexdigit())
                && !rest.is_empty()
                && rest
                    .chars()
                    .all(|c| is_unreserved(c) || is_sub_delim(c) || c == ':')
        });
        return Ok(valid.then(|| literal.to_string()));
    }
    let (address, zone) = match literal.split_once('%') {
        Some((address, zone)) => match zone.strip_prefix("25") {
            Some(zone) if !zone.is_empty() => (address, Some(zone)),
            _ => return Ok(None),
        },
        None => (literal, None),
    };
    if !is_ipv6_address(address) {
        return Ok(None);
    }
    match zone {
        None => Ok(Some(address.to_string())),
        Some(zone) if zone.chars().all(|c| is_unreserved(c) || c == '%') => {
            Ok(Some(format!("{address}%{}", decode_percent(zone)?)))
        }
        Some(_) => Ok(None),
    }
}

/// RFC 3986's `IPv6address`.
fn is_ipv6_address(address: &str) -> bool {
    fn groups(part: &str, ipv4_tail: bool) -> Option<usize> {
        if part.is_empty() {
            return Some(0);
        }
        let pieces: Vec<&str> = part.split(':').collect();
        let (last, init) = pieces.split_last()?;
        let h16 = |p: &&str| (1..=4).contains(&p.len()) && p.chars().all(|c| c.is_ascii_hexdigit());
        if !init.iter().all(h16) {
            return None;
        }
        if h16(last) {
            Some(pieces.len())
        } else if ipv4_tail && is_ipv4_address(last) {
            Some(pieces.len() + 1)
        } else {
            None
        }
    }
    match address.split_once("::") {
        Some((left, right)) => match (groups(left, false), groups(right, true)) {
            (Some(left), Some(right)) => left + right <= 7,
            _ => false,
        },
        None => groups(address, true) == Some(8),
    }
}

/// A dotted IPv4 address whose octets are written without leading zeros.
fn is_ipv4_address(address: &str) -> bool {
    let octets: Vec<&str> = address.split('.').collect();
    octets.len() == 4
        && octets.iter().all(|octet| {
            !octet.is_empty()
                && octet.len() <= 3
                && octet.chars().all(|c| c.is_ascii_digit())
                && (octet.len() == 1 || !octet.starts_with('0'))
                && octet.parse::<u8>().is_ok()
        })
}

pub(crate) struct EncodedPath<'p, S>(pub(crate) &'p [S]);

impl<S: AsRef<str>> Display for EncodedPath<'_, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, segment) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("/")?;
            }
            write!(f, "{}", encode_path_segment(segment.as_ref()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    /// Each `input` renders as `rendered`, as Nix 2.35.3 renders the `url` attribute of
    /// `builtins.parseFlakeRef "<input>"`.
    #[rstest]
    #[case(
        "https://flakehub.com/f/NixOS/nixpkgs/*",
        "https://flakehub.com/f/NixOS/nixpkgs/%2A"
    )]
    #[case("https://H.Org/A/b/../c", "https://H.Org/A/b/../c")]
    #[case("https://h/a%2Fb", "https://h/a%2Fb")]
    #[case(
        "https://h/a!$&()+,;=:@~b",
        "https://h/a%21%24%26%28%29%2B%2C%3B%3D:@~b"
    )]
    #[case("https://h/a'b", "https://h/a%27b")]
    #[case("https://u:p@h:8080/x", "https://u:p@h:8080/x")]
    #[case("https://u%40x@h/x", "https://u%40x@h/x")]
    #[case("https://h:443/x", "https://h:443/x")]
    #[case("https://h:/x", "https://h/x")]
    #[case("https://[::1]/x", "https://[::1]/x")]
    #[case("https://h/x/", "https://h/x/")]
    #[case("https://h", "https://h")]
    #[case("file:.", "file:.")]
    #[case("file:///abs/x", "file:///abs/x")]
    #[case("file:/abs", "file:/abs")]
    #[case("https://h/a%7Eb", "https://h/a~b")]
    #[case("https://h/a%41", "https://h/aA")]
    #[case("https://h/%e2%82%ac", "https://h/%E2%82%AC")]
    #[case("https://h/x?b=1&a=2", "https://h/x?a=2&b=1")]
    #[case("https://h/x?a=1&a=2", "https://h/x?a=1")]
    #[case("https://h/x?a", "https://h/x")]
    #[case("https://h/x?x=a%20b", "https://h/x?x=a%20b")]
    // A pair without `=` is ignored, but its escapes must still be well-formed. They need not
    // decode to UTF-8.
    #[case("https://h/x?a%41", "https://h/x")]
    #[case("https://h/x?%C3", "https://h/x")]
    // A query may hold `[` and `]` unescaped, and they render escaped.
    #[case("https://h/x?x=[1]", "https://h/x?x=%5B1%5D")]
    #[case("https://h/x?a]=b", "https://h/x?a%5D=b")]
    #[case("https://h/x?[", "https://h/x")]
    fn renders_like_nix(#[case] input: &str, #[case] rendered: &str) {
        assert_eq!(Url::parse(input).unwrap().to_string(), rendered);
    }

    #[rstest]
    #[case("https://h/r^x")]
    #[case("https://h/r{x")]
    #[case("https://h/r%zz")]
    #[case("https://h/r x")]
    #[case("https://h/r[x")]
    #[case("https://h/r|x")]
    #[case("https://h/r\"x")]
    #[case("file://localhost/x")]
    #[case("https://h:0/x")]
    #[case("https://h:65536/x")]
    #[case("https://h:x/y")]
    #[case("https://h/x?a%")]
    #[case("https://h/x?a%zz")]
    #[case("https://h/x?%2p&b=1")]
    fn rejects_out_of_grammar(#[case] input: &str) {
        assert!(Url::parse(input).is_err(), "{input}");
    }
}
