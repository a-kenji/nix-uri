use std::fmt::Display;

use nom::{
    branch::alt,
    bytes::complete::{tag, take_till},
    combinator::{opt, value},
    error::{context, VerboseError},
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
        let (rest, transport_type) = opt(TransportLayer::plus_parse)(rest)?;
        let (rest, _tag) = parse_sep(rest)?;
        let (res, location) = context("url location", take_till(|c| c == '#' || c == '?'))(rest)?;

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
        context(
            "resource selection",
            alt((
                value(Self::Git, tag("git")),
                value(Self::Mercurial, tag("hg")),
                value(Self::File, tag("file")),
                value(Self::Tarball, tag("tarball")),
            )),
        )(input)
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
