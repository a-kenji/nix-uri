use std::fmt::Display;

use serde::{Deserialize, Serialize};
use winnow::{
    combinator::{alt, preceded},
    PResult, Parser,
};

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
    pub fn parse(input: &mut &str) -> PResult<Self> {
        alt((
            "https".value(Self::Https),
            "http".value(Self::Http),
            "ssh".value(Self::Ssh),
            "file".value(Self::File),
        ))
        .parse_next(input)
    }
    pub fn plus_parse(input: &mut &str) -> PResult<Self> {
        preceded("+", Self::parse).parse_next(input)
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
        let mut uri = "+http://";
        let tp = TransportLayer::plus_parse(&mut uri).unwrap();
        assert_eq!(tp, TransportLayer::Http);
        assert_eq!(uri, "://");

        let mut uri = "+https://";
        let tp = TransportLayer::plus_parse(&mut uri).unwrap();
        assert_eq!(tp, TransportLayer::Https);
        assert_eq!(uri, "://");

        let mut uri = "+ssh://";
        let tp = TransportLayer::plus_parse(&mut uri).unwrap();
        assert_eq!(tp, TransportLayer::Ssh);
        assert_eq!(uri, "://");

        let mut uri = "+file://";
        let tp = TransportLayer::plus_parse(&mut uri).unwrap();
        assert_eq!(tp, TransportLayer::File);
        assert_eq!(uri, "://");

        // TODO: #158
        let mut uri = "://";
        let e = TransportLayer::plus_parse(&mut uri).unwrap_err();
        let expected_err = winnow::error::ContextError::new();
        let expected_err = winnow::error::ErrMode::Backtrack(expected_err);
        // let expected_err = winnow::error::ErrMode::Backtrack(InputError {
        //     input: "://",
        //     kind: ErrorKind::Tag,
        // });
        assert_eq!(expected_err, e);
    }

    // NOTE: at time of writing this comment, we use `nom`s `alt` combinator to parse `+....`. It
    // works more like a c-style switch-case than a rust `match`: This is to guard against
    // regressions, where we try and parse the `http` before `https`.
    #[test]
    fn http_s() {
        let mut http = "+httpfoobar";
        let mut https = "+httpsfoobar";
        let http_parsed = TransportLayer::plus_parse(&mut http).unwrap();
        let http_s_parsed = TransportLayer::plus_parse(&mut https).unwrap();

        let http_expected = TransportLayer::Http;
        let http_s_expected = TransportLayer::Https;

        assert_eq!("foobar", http);
        assert_eq!("foobar", http);
        assert_eq!(http_expected, http_parsed);
        assert_eq!(http_s_expected, http_s_parsed);
    }
}

#[cfg(test)]
mod err_msg {
    use super::*;
    #[test]
    #[ignore = "need to impl good error handling"]
    fn fizzbuzz() {
        let mut url = "+fizzbuzz";
        let _err = TransportLayer::plus_parse(&mut url).unwrap_err();
        todo!("Impl informative errors");
    }

    #[test]
    #[ignore = "need to impl good error handling"]
    fn missing_plus() {
        let mut url = "+";
        let _plus_err = TransportLayer::plus_parse(&mut url).unwrap_err();
        let _err = TransportLayer::parse(&mut "").unwrap_err();
        todo!("Impl informative errors");
    }
}
