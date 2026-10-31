use crate::{
    error::{NixUriError, NixUriResult},
    flakeref::{
        RenderUrl,
        attrs::{AttrReader, Attrs},
        forge::render_rev,
        url::Url,
        validators::matches_ref_shape,
    },
};

/// A Mercurial repository (`hg+http`, `hg+https`, `hg+ssh`, `hg+file`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MercurialRepo {
    pub url: String,
    pub ref_: Option<String>,
    pub rev: Option<String>,
    pub rev_count: Option<u64>,
    pub nar_hash: Option<String>,
    pub name: Option<String>,
}

/// The transports Nix's Mercurial fetcher takes.
pub(crate) const HG_TRANSPORTS: [&str; 4] = ["http", "https", "ssh", "file"];

impl MercurialRepo {
    pub fn new(url: String) -> Self {
        Self {
            url,
            ref_: None,
            rev: None,
            rev_count: None,
            nar_hash: None,
            name: None,
        }
    }

    pub(crate) fn from_url(mut url: Url) -> NixUriResult<Self> {
        let mut hg = Self::new(String::new());
        hg.ref_ = url.query.remove("ref").map(validated_hg_ref).transpose()?;
        hg.rev = url.query.remove("rev");
        url.strip_application();
        hg.url = url.to_string();
        Ok(hg)
    }

    pub(crate) fn from_attrs(mut attrs: AttrReader<'_>) -> NixUriResult<Self> {
        let url = attrs.required_string("url")?;
        Url::parse(&url)?;
        let hg = Self {
            url,
            ref_: attrs.string("ref")?.map(validated_hg_ref).transpose()?,
            rev: attrs.string("rev")?,
            rev_count: attrs.int("revCount")?,
            nar_hash: attrs.string("narHash")?,
            name: attrs.string("name")?,
        };
        attrs.finish()?;
        Ok(hg)
    }

    pub(crate) fn write_attrs(&self, attrs: &mut Attrs) {
        attrs.insert("type", "hg");
        attrs.insert("url", &self.url);
        attrs.set_opt("ref", self.ref_.as_ref());
        attrs.set_opt("rev", self.rev.as_ref());
        attrs.set_opt("revCount", self.rev_count);
        attrs.set_opt("narHash", self.nar_hash.as_ref());
        attrs.set_opt("name", self.name.as_ref());
    }

    /// `hg+<url>` with `ref` and `rev`.
    pub(crate) fn render(&self) -> RenderUrl<'_> {
        let mut out = RenderUrl::from_url_string(&self.url, |_| "hg+");
        out.query_opt("ref", self.ref_.as_deref());
        out.query_opt("rev", self.rev.as_deref().map(render_rev));
        out
    }
}

fn validated_hg_ref(value: String) -> NixUriResult<String> {
    if matches_ref_shape(&value) {
        return Ok(value);
    }
    Err(NixUriError::InvalidValue {
        field: "ref",
        reason: "expected a Mercurial branch or tag name".to_string(),
    })
}
