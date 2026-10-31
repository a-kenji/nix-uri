use crate::{
    error::NixUriResult,
    flakeref::{
        RenderUrl,
        attrs::{AttrReader, Attrs},
        url::Url,
        validators::render_nar_hash,
    },
};

/// A file or tarball downloaded from a URL: the Nix types `tarball` (`tarball+https://`, and a plain `https://` or `file:` URL) and `file` (`file+https://`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Download {
    pub url: String,
    pub nar_hash: Option<String>,
    pub rev: Option<String>,
    pub rev_count: Option<u64>,
    pub last_modified: Option<u64>,
    pub name: Option<String>,
    pub unpack: Option<bool>,
}

pub(crate) const DOWNLOAD_TRANSPORTS: [&str; 3] = ["http", "https", "file"];

impl Download {
    pub fn new(url: String) -> Self {
        Self {
            url,
            nar_hash: None,
            rev: None,
            rev_count: None,
            last_modified: None,
            name: None,
            unpack: None,
        }
    }

    /// `revCount` and `lastModified` that are not integers are dropped.
    pub(crate) fn from_url(mut url: Url) -> Self {
        let mut download = Self::new(String::new());
        download.nar_hash = url.query.get("narHash").cloned();
        download.rev = url.query.get("rev").cloned();
        download.rev_count = url.query.get("revCount").and_then(|n| n.parse().ok());
        download.last_modified = url.query.get("lastModified").and_then(|n| n.parse().ok());
        for key in [
            "url",
            "narHash",
            "name",
            "unpack",
            "rev",
            "revCount",
            "lastModified",
        ] {
            url.query.remove(key);
        }
        url.strip_application();
        download.url = url.to_string();
        download
    }

    pub(crate) fn from_attrs(mut attrs: AttrReader<'_>) -> NixUriResult<Self> {
        let url = attrs.required_string("url")?;
        Url::parse(&url)?;
        let download = Self {
            url,
            nar_hash: attrs.string("narHash")?,
            rev: attrs.string("rev")?,
            rev_count: attrs.int("revCount")?,
            last_modified: attrs.int("lastModified")?,
            name: attrs.string("name")?,
            unpack: attrs.bool("unpack")?,
        };
        attrs.finish()?;
        Ok(download)
    }

    pub(crate) fn write_attrs(&self, ty: &'static str, attrs: &mut Attrs) {
        attrs.insert("type", ty);
        attrs.insert("url", &self.url);
        attrs.set_opt("narHash", self.nar_hash.as_ref());
        attrs.set_opt("rev", self.rev.as_ref());
        attrs.set_opt("revCount", self.rev_count);
        attrs.set_opt("lastModified", self.last_modified);
        attrs.set_opt("name", self.name.as_ref());
        attrs.set_opt("unpack", self.unpack);
    }

    /// The bare URL with `narHash`.
    pub(crate) fn render(&self) -> RenderUrl {
        let mut out = RenderUrl::from_url_string(&self.url, |_| String::new());
        let nar_hash = self.nar_hash.as_deref().map(render_nar_hash);
        out.query_opt("narHash", nar_hash.as_deref());
        out
    }
}
