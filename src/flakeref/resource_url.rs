use std::fmt::Display;

use serde::{Deserialize, Serialize};
use winnow::{
    combinator::{alt, opt, trace},
    error::{StrContext, StrContextValue},
    token::take_till,
    PResult, Parser,
};

use crate::parser::parse_sep;

use super::TransportLayer;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceUrl {
    pub res_type: ResourceType,
    pub location: String,
    pub transport_type: Option<TransportLayer>,
}

impl ResourceUrl {
    pub fn parse(input: &mut &str) -> PResult<Self> {
        let res_type = ResourceType::parse(input)?;
        let transport_type = opt(TransportLayer::plus_parse).parse_next(input)?;
        let _tag = parse_sep(input)?;
        let location = take_till(0.., |c| c == '#' || c == '?')
            .context(StrContext::Expected(StrContextValue::StringLiteral(
                "Expected location",
            )))
            .parse_next(input)?;

        Ok(Self {
            res_type,
            location: location.to_string(),
            transport_type,
        })
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
    pub fn parse(input: &mut &str) -> PResult<Self> {
        alt((
            trace("tag: git", "git".value(Self::Git)),
            trace("tag: hg", "hg".value(Self::Mercurial)),
            trace("tag: file", "file".value(Self::File)),
            trace("tag: tarball", "tarball".value(Self::Tarball)),
        ))
        .context(StrContext::Expected(StrContextValue::StringLiteral(
            "git|hg|file|tarball",
        )))
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
        let mut url = "gitfoobar";
        let parsed = ResourceType::parse(&mut url).unwrap();
        let expected = ResourceType::Git;
        assert_eq!(expected, parsed);
        assert_eq!("foobar", url);
    }

    #[test]
    #[ignore = "TODO: handle colission between git and git<hub|lab> meaningfully"]
    fn github() {
        let mut url = "github";
        let parsed = ResourceType::parse(&mut url).unwrap();
        let expected = ResourceType::Git;
        assert_eq!(expected, parsed);
        assert_eq!("foobar", url);
    }

    #[test]
    #[ignore = "TODO: handle colission between git and git<hub|lab> meaningfully"]
    fn gitlab() {
        let mut url = "gitlab";
        let parsed = ResourceType::parse(&mut url).unwrap();
        let expected = ResourceType::Git;
        assert_eq!(expected, parsed);
        assert_eq!("foobar", url);
    }

    #[test]
    #[ignore = "need to impl good error handling"]
    fn gat() {
        let mut url = "gat";
        let _err = ResourceType::parse(&mut url).unwrap_err();
        todo!("Imple informative errors");
    }
}
