use crate::{
    error::{NixUriError, NixUriResult, UnsupportedReason},
    flakeref::{
        RenderUrl,
        attrs::{AttrReader, Attrs},
        url::{Url, render_segments},
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

    pub(crate) fn from_url(url: &Url) -> NixUriResult<Self> {
        if url.path.iter().any(|segment| segment.contains('/')) {
            return Err(NixUriError::InvalidUrl(url.to_string()));
        }
        let mut path = collapse_slashes(&url.path.join("/"));
        if path.contains('\0') {
            return Err(nul_in_path(&path));
        }
        if url.path.first().is_some_and(String::is_empty) && !path.starts_with('/') {
            path.insert(0, '/');
        }
        Ok(Self::new(path))
    }

    /// Apply the query of a path reference: `rev` and `narHash`, and the integers
    /// `revCount` and `lastModified`. Any other key is an error.
    pub(crate) fn apply_query<'a>(
        &mut self,
        query: impl IntoIterator<Item = (&'a String, &'a String)>,
    ) -> NixUriResult<()> {
        for (key, value) in query {
            match key.as_str() {
                "rev" => self.rev = Some(value.clone()),
                "narHash" => self.nar_hash = Some(value.clone()),
                "revCount" => self.rev_count = Some(require_integer("revCount", value)?),
                "lastModified" => {
                    self.last_modified = Some(require_integer("lastModified", value)?);
                }
                _ => {
                    return Err(NixUriError::Unsupported(UnsupportedReason::Param {
                        name: key.clone(),
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
    pub(crate) fn render(&self) -> RenderUrl {
        let mut out = RenderUrl::new(format!("path:{}", render_fs_path(&self.path)));
        out.query_opt("rev", self.rev.as_deref());
        out.query_opt("narHash", self.nar_hash.as_deref());
        out.query_opt("revCount", self.rev_count.map(|n| n.to_string()).as_deref());
        out.query_opt(
            "lastModified",
            self.last_modified.map(|n| n.to_string()).as_deref(),
        );
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

pub(crate) fn render_fs_path(path: &str) -> String {
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
    render_segments(&segments)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

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
        assert_eq!(super::render_fs_path(path), rendered);
    }
}
