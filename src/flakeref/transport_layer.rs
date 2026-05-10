use std::fmt::Display;

use serde::{Deserialize, Serialize};
use winnow::{
    ModalResult, Parser,
    combinator::{alt, cut_err, preceded},
    error::{StrContext, StrContextValue},
};

use crate::error::{NixUriError, tag};

/// Specifies the `+<layer>` component, e.g. `git+https://`.
///
/// `TransportLayer` has no sentinel "no transport" variant: the canonical way
/// to say "no transport" is `Option<TransportLayer>::None` on the consumer
/// (e.g. [`super::ResourceUrl::transport_type`]). For the same reason there
/// is no `Default` impl; there is no sensible default transport.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransportLayer {
    Http,
    Https,
    Ssh,
    File,
}

impl TransportLayer {
    #[allow(dead_code)]
    pub(crate) fn parse(input: &mut &str) -> ModalResult<Self> {
        alt((
            tag("https").value(Self::Https),
            tag("http").value(Self::Http),
            tag("ssh").value(Self::Ssh),
            tag("file").value(Self::File),
        ))
        .context(StrContext::Label("transport type"))
        .parse_next(input)
    }
    #[allow(dead_code)]
    pub(crate) fn plus_parse(input: &mut &str) -> ModalResult<Self> {
        preceded(
            '+'.context(StrContext::Expected(StrContextValue::CharLiteral('+'))),
            cut_err(Self::parse),
        )
        .context(StrContext::Label("transport type separator"))
        .parse_next(input)
    }
}

impl TryFrom<&str> for TransportLayer {
    type Error = NixUriError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "http" => Ok(Self::Http),
            "https" => Ok(Self::Https),
            "ssh" => Ok(Self::Ssh),
            "file" => Ok(Self::File),
            // The empty string used to map to `Self::None`; without that
            // sentinel an empty transport is just an unknown one.
            err => Err(NixUriError::Unsupported(
                crate::error::UnsupportedReason::TransportLayer { ty: err.into() },
            )),
        }
    }
}

impl Display for TransportLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http => write!(f, "http"),
            Self::Https => write!(f, "https"),
            Self::Ssh => write!(f, "ssh"),
            Self::File => write!(f, "file"),
        }
    }
}

#[cfg(test)]
mod inc_parse {
    use cool_asserts::assert_matches;

    use super::*;

    #[test]
    fn try_from_colon_slashes_is_unsupported() {
        let err = TransportLayer::try_from("://").unwrap_err();
        assert_matches!(
            err,
            NixUriError::Unsupported(crate::error::UnsupportedReason::TransportLayer { ty })
                => assert_eq!(ty, "://")
        );
    }

    // NOTE: at time of writing this comment, `alt` parses `+....` like a c-style
    // switch-case rather than a rust `match`: longest prefix wins. This guards
    // against regressions where we try to parse `http` before `https`.
    #[test]
    fn http_s() {
        let http = "+httpfoobar";
        let https = "+httpsfoobar";
        let (rest, http_parsed) = TransportLayer::plus_parse.parse_peek(http).unwrap();
        assert_eq!("foobar", rest);
        let (rest, https_parsed) = TransportLayer::plus_parse.parse_peek(https).unwrap();
        let http_expected = TransportLayer::Http;
        let http_s_expected = TransportLayer::Https;
        assert_eq!(http_expected, http_parsed);
        assert_eq!(http_s_expected, https_parsed);
        assert_eq!("foobar", rest);
    }
}

#[cfg(test)]
mod err_msg {
    use cool_asserts::assert_matches;

    use super::*;

    #[test]
    fn try_from_empty_is_unsupported() {
        // The empty string used to map to TransportLayer::None; now that the
        // sentinel variant is gone, an empty transport is an error rather
        // than a quietly-defaulted value.
        let err = TransportLayer::try_from("").unwrap_err();
        assert_matches!(
            err,
            NixUriError::Unsupported(crate::error::UnsupportedReason::TransportLayer { ty })
                => assert_eq!(ty, "")
        );
    }

    #[test]
    fn unknown_transport_after_plus_routes_to_transport_layer_unsupported() {
        use crate::parser::parse_nix_uri;

        assert_matches!(
            parse_nix_uri("git+fizzbuzz://x"),
            Err(NixUriError::Unsupported(crate::error::UnsupportedReason::TransportLayer { ty }))
                => assert_eq!(ty, "fizzbuzz")
        );
    }
}
