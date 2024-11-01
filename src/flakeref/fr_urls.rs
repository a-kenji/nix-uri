use std::{fmt::Display, path::Path};

use nom::{
    branch::alt,
    bytes::complete::{tag, take_until},
    combinator::{map, opt, rest},
    IResult,
};
use serde::{Deserialize, Serialize};

use crate::{
    error::{NixUriError, NixUriResult},
    parser::parse_url_type,
};
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum UrlType {
    #[default]
    None,
    Https,
    Ssh,
    File,
}

impl UrlType {
    /// TODO: refactor so None is not in UrlType. Use Option to encapsulate this
    pub fn parse_file(input: &str) -> IResult<&str, Self> {
        alt((
            map(tag(""), |_| UrlType::None),
            map(tag("https"), |_| UrlType::Https),
            map(tag("ssh"), |_| UrlType::Ssh),
            map(tag("file"), |_| UrlType::File),
        ))(input)
    }
}

impl TryFrom<&str> for UrlType {
    type Error = NixUriError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        use UrlType::*;
        match value {
            "" => Ok(None),
            "https" => Ok(Https),
            "ssh" => Ok(Ssh),
            "file" => Ok(File),
            err => Err(NixUriError::UnknownUrlType(err.into())),
        }
    }
}

impl Display for UrlType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UrlType::None => write!(f, "No Url Type Specified"),
            UrlType::Https => write!(f, "https"),
            UrlType::Ssh => write!(f, "ssh"),
            UrlType::File => write!(f, "file"),
        }
    }
}
