use std::fmt::Display;

use serde::{Deserialize, Serialize};

/// A non-empty vector of strings that make up the attribute path
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttrPath {
    attrs: Vec<String>
}


impl Display for AttrPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.attrs.as_slice().join("."))
    }
}
