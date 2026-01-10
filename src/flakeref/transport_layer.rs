use std::fmt::Display;

use nom::{
    IResult, Parser,
    branch::alt,
    character::complete::char,
    combinator::{cut, value},
    error::context,
    sequence::preceded,
};
use serde::{Deserialize, Serialize};

use crate::{
    IErr,
    error::{NixUriError, tag},
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
    /// TODO: refactor so None is not in `TransportLayer`. Use Option to encapsulate this
    pub fn parse(input: &str) -> IResult<&str, Self, IErr<&str>> {
        context(
            "transport type",
            alt((
                value(Self::Https, tag("https")),
                value(Self::Http, tag("http")),
                value(Self::Ssh, tag("ssh")),
                value(Self::File, tag("file")),
            )),
        )
        .parse(input)
    }
    pub fn plus_parse(input: &str) -> IResult<&str, Self, IErr<&str>> {
        context(
            "transport type separator",
            preceded(char('+'), cut(Self::parse)),
        )
        .parse(input)
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
    use cool_asserts::assert_matches;

    use super::*;
    use crate::error::{BaseErrorKind, ErrorTree, Expectation, StackContext};

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

        assert_matches!(
            e,
            ErrorTree::Stack {
                base, //: Box(ErrorTree::Base {location, kind}),
                contexts,
            } => {
                assert_matches!(*base, ErrorTree::Base {
                    location: "://",
                    kind: BaseErrorKind::Expected(Expectation::Char('+'))
                });
                assert_eq!(contexts, [("://", StackContext::Context("transport type separator"))]);
            }
        );
        // panic!("{:#?}", e);
        // panic!("{:#?}", e);
        //todo: verify the error structure
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
    use cool_asserts::assert_matches;
    use nom::Finish;

    use super::*;
    use crate::error::{BaseErrorKind, ErrorTree, Expectation, StackContext};

    #[test]
    fn fizzbuzz() {
        let url = "+fizzbuzz";
        let err = TransportLayer::plus_parse(url).finish().unwrap_err();
        // panic!("{:?}", err);
        assert_matches!(
            err,
            ErrorTree::Stack {
                base,
                contexts
            } => {
                // TODO: use assert-matches idioms nicely
                assert_matches!(*base, ErrorTree::Alt (alts) => {
                    for alt in alts {
                        assert_matches!(
                            alt,
                            ErrorTree::Base{
                                location: "fizzbuzz",
                                kind: BaseErrorKind::Expected(Expectation::Tag(
                                    "https" |
                                    "http" |
                                    "ssh" |
                                    "file"
                                ))
                            }
                        )
                    };
                });
                assert_eq!(
                    contexts,
                    [
                        ("fizzbuzz", StackContext::Kind(nom::error::ErrorKind::Alt)),
                        ("fizzbuzz", StackContext::Context("transport type")),
                        ("+fizzbuzz", StackContext::Context("transport type separator"))
                    ]
                );
            }
        );
    }

    #[test]
    fn missing_plus() {
        let url = "+";
        let _plus_err = TransportLayer::plus_parse(url).finish().unwrap_err();
        let err = TransportLayer::parse("").finish().unwrap_err();
        assert_matches!(
            err,
            ErrorTree::Stack {
                base,
                contexts
            } => {
                // TODO: use assert-matches idioms nicely
                assert_matches!(*base, ErrorTree::Alt (alts) => {
                    for alt in alts {
                        assert_matches!(
                            alt,
                            ErrorTree::Base{
                                location: "",
                                kind: BaseErrorKind::Expected(Expectation::Tag(
                                    "https" |
                                    "http" |
                                    "ssh" |
                                    "file"
                                ))
                            }
                        )
                    };
                });
                assert_eq!(
                    contexts,
                    [
                        ("", StackContext::Kind(nom::error::ErrorKind::Alt)),
                        ("", StackContext::Context("transport type"))
                    ]
                );
            }
        );
    }
}
