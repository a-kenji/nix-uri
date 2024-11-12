use std::fmt::Display;

use nom::{
    branch::alt, bytes::complete::tag, character::complete::char, combinator::value,
    sequence::preceded, IResult,
};
use serde::{Deserialize, Serialize};

use crate::error::NixUriError;

/// Specifies the `+<layer>` component, e.g. `git+https://`
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TransportLayer {
    #[default]
    None,
    Http,
    Https,
    Ssh,
    File,
}

impl TransportLayer {
    /// TODO: refactor so None is not in `TransportLayer`. Use Option to encapsulate this
    pub fn parse(input: &str) -> IResult<&str, Self> {
        alt((
            value(Self::Https, tag("https")),
            value(Self::Http, tag("http")),
            value(Self::Ssh, tag("ssh")),
            value(Self::File, tag("file")),
        ))(input)
    }
    pub fn plus_parse(input: &str) -> IResult<&str, Self> {
        preceded(char('+'), Self::parse)(input)
    }
}

impl TryFrom<&str> for TransportLayer {
    type Error = NixUriError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "" => Ok(Self::None),
            "http" => Ok(Self::Http),
            "https" => Ok(Self::Https),
            "ssh" => Ok(Self::Ssh),
            "file" => Ok(Self::File),
            err => Err(NixUriError::UnknownTransportLayer(err.into())),
        }
    }
}

impl Display for TransportLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "No Url Type Specified"),
            Self::Http => write!(f, "http"),
            Self::Https => write!(f, "https"),
            Self::Ssh => write!(f, "ssh"),
            Self::File => write!(f, "file"),
        }
    }
}

#[cfg(test)]
mod inc_parse {
    use super::*;
    #[test]
    fn basic() {
        let uri = "+http://";
        let (rest, tp) = TransportLayer::plus_parse(uri).unwrap();
        assert_eq!(tp, TransportLayer::Http);
        assert_eq!(rest, "://");

        let uri = "+https://";
        let (rest, tp) = TransportLayer::plus_parse(uri).unwrap();
        assert_eq!(tp, TransportLayer::Https);
        assert_eq!(rest, "://");

        let uri = "+ssh://";
        let (rest, tp) = TransportLayer::plus_parse(uri).unwrap();
        assert_eq!(tp, TransportLayer::Ssh);
        assert_eq!(rest, "://");

        let uri = "+file://";
        let (rest, tp) = TransportLayer::plus_parse(uri).unwrap();
        assert_eq!(tp, TransportLayer::File);
        assert_eq!(rest, "://");

        // TODO: #158
        let uri = "://";
        let nom::Err::Error(e) = TransportLayer::plus_parse(uri).unwrap_err() else {
            panic!();
        };
        assert_eq!(e.input, "://");
    }

    // NOTE: at time of writing this comment, we use `nom`s `alt` combinator to parse `+....`. It
    // works more like a c-style switch-case than a rust `match`: This is to guard against
    // regressions, where we try and parse the `http` before `https`.
    #[test]
    fn http_s() {
        let http = "+httpfoobar";
        let https = "+httpsfoobar";
        let (rest, http_parsed) = TransportLayer::plus_parse(http).unwrap();
        assert_eq!("foobar", rest);
        let (rest, https_parsed) = TransportLayer::plus_parse(https).unwrap();
        let http_expected = TransportLayer::Http;
        let http_s_expected = TransportLayer::Https;
        assert_eq!(http_expected, http_parsed);
        assert_eq!(http_s_expected, https_parsed);
        assert_eq!("foobar", rest);
    }
}

#[cfg(test)]
mod err_msg {
    use super::*;
    #[test]
    #[ignore = "need to impl good error handling"]
    fn fizzbuzz() {
        let url = "+fizzbuzz";
        let _err = TransportLayer::plus_parse(url).unwrap_err();
        todo!("Impl informative errors");
    }

    #[test]
    #[ignore = "need to impl good error handling"]
    fn missing_plus() {
        let url = "+";
        let _plus_err = TransportLayer::plus_parse(url).unwrap_err();
        let _err = TransportLayer::parse("").unwrap_err();
        todo!("Impl informative errors");
    }
}
