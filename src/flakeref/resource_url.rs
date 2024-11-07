use std::fmt::Display;

use nom::{
    branch::alt,
    bytes::complete::{tag, take_till},
    combinator::{map, opt},
    error::{VerboseError, VerboseErrorKind},
    IResult,
};
use serde::{Deserialize, Serialize};

use crate::parser::parse_sep;

use super::TransportLayer;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceUrl {
    pub res_type: ResourceType,
    pub location: String,
    pub transport_type: Option<TransportLayer>,
}

impl ResourceUrl {
    pub fn parse(input: &str) -> IResult<&str, Self, VerboseError<&str>> {
        let (rest, res_type) = ResourceType::parse(input)?;
        // TODO: ensure context is passed up: "+foobar" gives context that "foobar" isn't valid
        let (rest, transport_type) = opt(TransportLayer::parse_plus)(rest)?;
        let (rest, _tag) = parse_sep(rest)?;
        let (res, location) = take_till(|c| c == '#' || c == '?')(rest)?;

        Ok((
            res,
            Self {
                res_type,
                location: location.to_string(),
                transport_type,
            },
        ))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ResourceType {
    Git,
    Mercurial,
    File,
    Tarball,
}

impl ResourceType {
    pub fn parse(input: &str) -> IResult<&str, Self, VerboseError<&str>> {
        let res: Result<(&str, Self), nom::Err<nom::error::Error<&str>>> = alt((
            map(tag("git"), |_| Self::Git),
            map(tag("hg"), |_| Self::Mercurial),
            map(tag("file"), |_| Self::File),
            map(tag("tarball"), |_| Self::Tarball),
        ))(input);
        res.map_err(|e| {
            e.map(|inner| {
                let new_input = &inner.input[..7];
                VerboseError {
                    errors: vec![(new_input, VerboseErrorKind::Context("unrecognised type"))],
                }
            })
        })
    }
}

impl Display for ResourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let out_str = match self {
            Self::Git => "git",
            Self::Mercurial => "mercurial",
            Self::File => "file",
            Self::Tarball => "tarball",
        };
        write!(f, "{}", out_str)
    }
}

#[cfg(test)]
mod res_url {
    use super::*;

    #[test]
    fn git() {
        let url = "gitfoobar";
        let (rest, parsed) = ResourceType::parse(url).unwrap();
        let expected = ResourceType::Git;
        assert_eq!(expected, parsed);
        assert_eq!("foobar", rest);
    }

    #[test]
    #[ignore = "TODO: handle colission between git and git<hub|lab> meaningfully"]
    fn github() {
        let url = "github";
        let (rest, parsed) = ResourceType::parse(url).unwrap();
        let expected = ResourceType::Git;
        assert_eq!(expected, parsed);
        assert_eq!("foobar", rest);
    }

    #[test]
    #[ignore = "TODO: handle colission between git and git<hub|lab> meaningfully"]
    fn gitlab() {
        let url = "gitlab";
        let (rest, parsed) = ResourceType::parse(url).unwrap();
        let expected = ResourceType::Git;
        assert_eq!(expected, parsed);
        assert_eq!("foobar", rest);
    }

    #[test]
    #[ignore = "need to impl good error handling"]
    fn gat() {
        let url = "gat";
        let _err = ResourceType::parse(url).unwrap_err();
        todo!("Imple informative errors");
    }
}
