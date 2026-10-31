use std::collections::BTreeMap;

use crate::error::{NixUriError, NixUriResult, UnsupportedReason};

/// A flake reference as an attribute set: what `builtins.parseFlakeRef` returns, a node
/// in `flake.lock`, or a registry entry.
///
/// With the `serde` feature it serialises to the same JSON as those: `{"type": "github", "owner": "NixOS", "repo": "nixpkgs"}`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Attrs(BTreeMap<String, AttrValue>);

/// The value of one attribute.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
#[non_exhaustive]
pub enum AttrValue {
    String(String),
    Int(u64),
    Bool(bool),
}

impl Attrs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, name: &str) -> Option<&AttrValue> {
        self.0.get(name)
    }

    /// Set `name` to `value`, returning the previous value.
    pub fn insert(
        &mut self,
        name: impl Into<String>,
        value: impl Into<AttrValue>,
    ) -> Option<AttrValue> {
        self.0.insert(name.into(), value.into())
    }

    pub fn remove(&mut self, name: &str) -> Option<AttrValue> {
        self.0.remove(name)
    }

    /// The attributes, sorted by name.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &AttrValue)> {
        self.0.iter().map(|(name, value)| (name.as_str(), value))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn set_opt(&mut self, name: &str, value: Option<impl Into<AttrValue>>) {
        if let Some(value) = value {
            self.insert(name, value);
        }
    }
}

impl<K: Into<String>, V: Into<AttrValue>> FromIterator<(K, V)> for Attrs {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self(
            iter.into_iter()
                .map(|(name, value)| (name.into(), value.into()))
                .collect(),
        )
    }
}

impl From<BTreeMap<String, AttrValue>> for Attrs {
    fn from(map: BTreeMap<String, AttrValue>) -> Self {
        Self(map)
    }
}

impl From<Attrs> for BTreeMap<String, AttrValue> {
    fn from(attrs: Attrs) -> Self {
        attrs.0
    }
}

impl From<String> for AttrValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for AttrValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_string())
    }
}

impl From<&String> for AttrValue {
    fn from(value: &String) -> Self {
        Self::String(value.clone())
    }
}

impl From<u64> for AttrValue {
    fn from(value: u64) -> Self {
        Self::Int(value)
    }
}

impl From<bool> for AttrValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

/// Reads typed attributes out of an [`Attrs`] for one scheme, and checks at the end that
/// no attribute was left over.
pub(crate) struct AttrReader<'a> {
    scheme: &'static str,
    attrs: BTreeMap<&'a str, &'a AttrValue>,
}

impl<'a> AttrReader<'a> {
    /// `type`, `dir` and `__final` are taken off before the scheme reads its own
    /// attributes. `__final`, a marker in lock files, is ignored.
    pub(crate) fn new(scheme: &'static str, attrs: &'a Attrs) -> Self {
        let attrs = attrs
            .iter()
            .filter(|(name, _)| !matches!(*name, "type" | "dir" | "__final"))
            .collect();
        Self { scheme, attrs }
    }

    pub(crate) fn string(&mut self, name: &'static str) -> NixUriResult<Option<String>> {
        match self.attrs.remove(name) {
            None => Ok(None),
            Some(AttrValue::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(wrong_type(name, "a string")),
        }
    }

    pub(crate) fn required_string(&mut self, name: &'static str) -> NixUriResult<String> {
        self.string(name)?.ok_or_else(|| {
            NixUriError::Unsupported(UnsupportedReason::MissingParameter {
                ty: self.scheme.to_string(),
                parameter: name.to_string(),
            })
        })
    }

    pub(crate) fn int(&mut self, name: &'static str) -> NixUriResult<Option<u64>> {
        match self.attrs.remove(name) {
            None => Ok(None),
            Some(AttrValue::Int(n)) => Ok(Some(*n)),
            Some(_) => Err(wrong_type(name, "an integer")),
        }
    }

    pub(crate) fn bool(&mut self, name: &'static str) -> NixUriResult<Option<bool>> {
        match self.attrs.remove(name) {
            None => Ok(None),
            Some(AttrValue::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(wrong_type(name, "a Boolean")),
        }
    }

    /// Fail on the first attribute the scheme does not take.
    pub(crate) fn finish(self) -> NixUriResult<()> {
        match self.attrs.into_keys().next() {
            None => Ok(()),
            Some(name) => Err(NixUriError::Unsupported(UnsupportedReason::Param {
                name: name.to_string(),
            })),
        }
    }
}

fn wrong_type(field: &'static str, expected: &str) -> NixUriError {
    NixUriError::InvalidValue {
        field,
        reason: format!("expected {expected}"),
    }
}

/// JSON without the `serde` feature, so tests can compare attributes with Nix's output in any build.
#[cfg(test)]
impl Attrs {
    pub(crate) fn to_json(&self) -> String {
        let object: serde_json::Map<String, serde_json::Value> = self
            .iter()
            .map(|(name, value)| {
                let value = match value {
                    AttrValue::String(s) => s.as_str().into(),
                    AttrValue::Int(n) => (*n).into(),
                    AttrValue::Bool(b) => (*b).into(),
                };
                (name.to_string(), value)
            })
            .collect();
        serde_json::Value::Object(object).to_string()
    }

    pub(crate) fn from_json(json: &str) -> Self {
        let serde_json::Value::Object(object) = serde_json::from_str(json).unwrap() else {
            panic!("not a JSON object: {json}");
        };
        object
            .into_iter()
            .map(|(name, value)| {
                let value = match value {
                    serde_json::Value::String(s) => AttrValue::String(s),
                    serde_json::Value::Number(n) => AttrValue::Int(n.as_u64().unwrap()),
                    serde_json::Value::Bool(b) => AttrValue::Bool(b),
                    other => panic!("not an attribute value: {other}"),
                };
                (name, value)
            })
            .collect()
    }
}

#[cfg(test)]
#[cfg(feature = "serde")]
mod tests {
    use super::*;

    #[test]
    fn json_shape_is_a_lock_file_node() {
        let attrs: Attrs = [
            ("type", AttrValue::from("github")),
            ("owner", "NixOS".into()),
            ("lastModified", 1_700_000_000u64.into()),
            ("submodules", true.into()),
        ]
        .into_iter()
        .collect();
        let json = serde_json::to_string(&attrs).unwrap();
        assert_eq!(
            json,
            r#"{"lastModified":1700000000,"owner":"NixOS","submodules":true,"type":"github"}"#
        );
        assert_eq!(serde_json::from_str::<Attrs>(&json).unwrap(), attrs);
    }
}
