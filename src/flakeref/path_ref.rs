use std::borrow::Cow;

use crate::{
    error::{NixUriError, NixUriResult, UnsupportedReason},
    flakeref::{
        RenderUrl,
        attrs::{AttrReader, Attrs},
        url::{Query, Url},
    },
};

/// A directory on the local filesystem (`path:`, or a bare path).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PathRef {
    pub path: String,
    pub rev: Option<String>,
    pub rev_count: Option<u64>,
    pub last_modified: Option<u64>,
    pub nar_hash: Option<String>,
}

impl PathRef {
    pub fn new(path: String) -> Self {
        Self {
            path,
            rev: None,
            rev_count: None,
            last_modified: None,
            nar_hash: None,
        }
    }

    /// The path of a bare path reference.
    pub(crate) fn from_bare(path: &str) -> Self {
        Self::new(collapse_slashes(path))
    }

    pub(crate) fn from_url(url: &Url<'_>) -> NixUriResult<Self> {
        if url.path.iter().any(|segment| segment.contains('/')) {
            return Err(NixUriError::InvalidUrl(url.to_string()));
        }
        let mut path = collapse_slashes(&url.path.join("/"));
        if path.contains('\0') {
            return Err(nul_in_path(&path));
        }
        if url.path.first().is_some_and(|s| s.is_empty()) && !path.starts_with('/') {
            path.insert(0, '/');
        }
        Ok(Self::new(path))
    }

    /// Apply the query of a path reference: `rev` and `narHash`, and the integers
    /// `revCount` and `lastModified`. Any other key is an error.
    pub(crate) fn apply_query(&mut self, query: &Query<'_>) -> NixUriResult<()> {
        for (key, value) in query {
            match key.as_ref() {
                "rev" => self.rev = Some(value.to_string()),
                "narHash" => self.nar_hash = Some(value.to_string()),
                "revCount" => self.rev_count = Some(require_integer("revCount", value)?),
                "lastModified" => {
                    self.last_modified = Some(require_integer("lastModified", value)?);
                }
                _ => {
                    return Err(NixUriError::Unsupported(UnsupportedReason::Param {
                        name: key.to_string(),
                    }));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn from_attrs(mut attrs: AttrReader<'_>) -> NixUriResult<Self> {
        let path = Self {
            path: attrs.required_string("path")?,
            rev: attrs.string("rev")?,
            rev_count: attrs.int("revCount")?,
            last_modified: attrs.int("lastModified")?,
            nar_hash: attrs.string("narHash")?,
        };
        attrs.finish()?;
        Ok(path)
    }

    pub(crate) fn write_attrs(&self, attrs: &mut Attrs) {
        attrs.insert("type", "path");
        attrs.insert("path", &self.path);
        attrs.set_opt("rev", self.rev.as_ref());
        attrs.set_opt("revCount", self.rev_count);
        attrs.set_opt("lastModified", self.last_modified);
        attrs.set_opt("narHash", self.nar_hash.as_ref());
    }

    /// `path:<path>` with the path percent-encoded and every attribute in the query,
    /// values as given.
    pub(crate) fn render(&self) -> RenderUrl<'_> {
        let segments = fs_path_segments(&self.path)
            .into_iter()
            .map(Cow::Borrowed)
            .collect();
        let mut out = RenderUrl::from_segments("path", segments);
        out.query_opt("rev", self.rev.as_deref());
        out.query_opt("narHash", self.nar_hash.as_deref());
        out.query_opt("revCount", self.rev_count.map(|n| n.to_string()));
        out.query_opt("lastModified", self.last_modified.map(|n| n.to_string()));
        out
    }
}

/// The path of a path reference cannot contain a NUL byte.
pub(crate) fn nul_in_path(path: &str) -> NixUriError {
    NixUriError::InvalidUrl(format!("path '{path}' contains a NUL byte"))
}

fn require_integer(field: &'static str, value: &str) -> NixUriResult<u64> {
    value.parse().map_err(|_| NixUriError::InvalidValue {
        field,
        reason: "expected a non-negative integer".to_string(),
    })
}

fn collapse_slashes(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        if c == '/' && out.ends_with('/') {
            continue;
        }
        out.push(c);
    }
    out
}

pub(crate) fn fs_path_segments(path: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    if path.starts_with('/') {
        segments.push("");
    }
    let relative = path.trim_start_matches('/');
    segments.extend(relative.split('/').filter(|s| !s.is_empty()));
    if !relative.is_empty() && relative.ends_with('/') {
        segments.push("");
    }
    if path.is_empty() || path.ends_with('/') {
        segments.push("");
    }
    segments
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use crate::flakeref::url::EncodedPath;

    /// `builtins.flakeRefToString { type = "path"; path = <path>; }` in Nix 2.35.3.
    #[rstest]
    #[case("/", "/")]
    #[case("", "")]
    #[case("/a/b/", "/a/b//")]
    #[case("/a//b", "/a/b")]
    #[case("./foo", "./foo")]
    #[case("foo", "foo")]
    #[case("github:NixOS/nixpkgs", "github:NixOS/nixpkgs")]
    #[case("/a b%", "/a%20b%25")]
    #[case("/a/../b", "/a/../b")]
    fn renders_like_nix(#[case] path: &str, #[case] rendered: &str) {
        let segments = super::fs_path_segments(path);
        assert_eq!(EncodedPath(&segments).to_string(), rendered);
    }
}
