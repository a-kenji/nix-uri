use std::fmt::Display;

use nom::{
    branch::alt,
    bytes::complete::{tag, take_till},
    combinator::{map, opt},
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
    pub fn parse(input: &str) -> IResult<&str, Self> {
        let (rest, res_type) = ResourceType::parse(input)?;
        let (rest, transport_type) = opt(TransportLayer::plus_parse)(rest)?;
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
    pub fn parse(input: &str) -> IResult<&str, Self> {
        alt((
            map(tag("git"), |_| Self::Git),
            map(tag("hg"), |_| Self::Mercurial),
            map(tag("file"), |_| Self::File),
            map(tag("tarball"), |_| Self::Tarball),
        ))(input)
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
