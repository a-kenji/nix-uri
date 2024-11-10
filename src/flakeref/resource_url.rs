use std::fmt::Display;

use serde::{Deserialize, Serialize};
use winnow::{token::take_till, combinator::{alt, opt }, PResult, Parser};

use crate::parser::parse_sep;

use super::TransportLayer;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceUrl {
    pub res_type: ResourceType,
    pub location: String,
    pub transport_type: Option<TransportLayer>,
}

impl ResourceUrl {
    pub fn parse<'i>(input: &mut &'i str) -> PResult<Self> {
        let res_type = ResourceType::parse(input)?;
        let transport_type = opt(TransportLayer::plus_parse).parse_next(input)?;
        let _tag = parse_sep(input)?;
        let location = take_till(0.., |c| c == '#' || c == '?').parse_next(input)?;

        Ok(
            Self {
                res_type,
                location: location.to_string(),
                transport_type,
            },
        )
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
    pub fn parse<'i>(input: &mut &'i str) -> PResult<Self> {
        alt((
            "git".value(Self::Git),
            "hg".value(Self::Mercurial),
            "file".value(Self::File),
            "tarball".value(Self::Tarball),
        ))
        .parse_next(input)
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
