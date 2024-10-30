use std::fmt::Display;

use nom::{
    bytes::complete::{tag, take_till},
    IResult,
};
use serde::{Deserialize, Serialize};

use crate::NixUriError;

/// A non-empty vector of strings that make up the attribute path
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttrPath {
    pub(crate) attrs: Vec<String>,
}

impl Display for AttrPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.attrs.as_slice().join("."))
    }
}

impl std::str::FromStr for AttrPath {
    type Err = NixUriError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.contains('"') || s.contains('#') || s.is_empty() {
            return Err(NixUriError::InvalidAttrPath(s.to_string()));
        }
        let attrs = s.split('.').map(|s| s.to_string()).collect::<Vec<_>>();
        Ok(Self { attrs })
    }
}
