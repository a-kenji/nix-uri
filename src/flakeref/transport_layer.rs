use std::{fmt::Display, path::Path};

use nom::{
    branch::alt,
    bytes::complete::{tag, take_until},
    combinator::{map, opt, rest},
    sequence::preceded,
    IResult,
};
use serde::{Deserialize, Serialize};

use crate::{
    error::{NixUriError, NixUriResult},
    parser::parse_transport_type,
};

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
    /// TODO: refactor so None is not in TransportLayer. Use Option to encapsulate this
    pub fn parse(input: &str) -> IResult<&str, Self> {
        alt((
            map(tag("https"), |_| TransportLayer::Https),
            map(tag("http"), |_| TransportLayer::Http),
            map(tag("ssh"), |_| TransportLayer::Ssh),
            map(tag("file"), |_| TransportLayer::File),
        ))(input)
    }
    pub fn plus_parse(input: &str) -> IResult<&str, Self> {
        preceded(tag("+"), Self::parse)(input)
    }
}

impl TryFrom<&str> for TransportLayer {
    type Error = NixUriError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        use TransportLayer::*;
        match value {
            "" => Ok(None),
            "http" => Ok(Http),
            "https" => Ok(Https),
            "ssh" => Ok(Ssh),
            "file" => Ok(File),
            err => Err(NixUriError::UnknownTransportLayer(err.into())),
        }
    }
}

impl Display for TransportLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportLayer::None => write!(f, "No Url Type Specified"),
            TransportLayer::Http => write!(f, "http"),
            TransportLayer::Https => write!(f, "https"),
            TransportLayer::Ssh => write!(f, "ssh"),
            TransportLayer::File => write!(f, "file"),
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
    // regression tests, where we try and parse the `http` before `https`.
    #[test]
    fn http_s() {
        let http = "+httpfoobar";
        let https = "+httpsfoobar";
        let (rest, http_parsed) = TransportLayer::plus_parse(http).unwrap();
        let (rest, https_parsed) = TransportLayer::plus_parse(https).unwrap();
        let http_expected = TransportLayer::Http;
        let https_expected = TransportLayer::Https;
        assert_eq!(http_expected, http_parsed);
        assert_eq!(https_expected, https_parsed);
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
        let err = TransportLayer::plus_parse(url).unwrap_err();
        todo!("Impl informative errors");
    }

    #[test]
    #[ignore = "need to impl good error handling"]
    fn missing() {
        let url = "+";
        let plus_err = TransportLayer::plus_parse(url).unwrap_err();
        let err = TransportLayer::parse("").unwrap_err();
        assert_eq!(plus_err, err);
        // todo!("Impl informative errors");
    }
}

