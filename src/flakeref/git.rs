use std::borrow::Cow;
use std::net::Ipv6Addr;

use crate::{
    error::{NixUriError, NixUriResult},
    flakeref::{
        RenderUrl,
        attrs::{AttrReader, Attrs},
        forge::render_rev,
        url::{Authority, EncodedPath, Host, Url},
        validators::{query_bool, validated_ref_name},
    },
};

/// A Git repository (`git+https://`, `git+ssh://`, `git+file:`, `git://`, any `git+<transport>`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GitRepo {
    pub url: String,
    pub ref_: Option<String>,
    pub rev: Option<String>,
    pub shallow: Option<bool>,
    pub submodules: Option<bool>,
    pub lfs: Option<bool>,
    pub export_ignore: Option<bool>,
    pub all_refs: Option<bool>,
    pub verify_commit: Option<bool>,
    pub keytype: Option<String>,
    pub public_key: Option<String>,
    pub public_keys: Option<String>,
    pub nar_hash: Option<String>,
    pub rev_count: Option<u64>,
    pub last_modified: Option<u64>,
    pub name: Option<String>,
    pub dirty_rev: Option<String>,
    pub dirty_short_rev: Option<String>,
}

impl GitRepo {
    #[must_use]
    pub fn new(url: String) -> Self {
        Self {
            url,
            ref_: None,
            rev: None,
            shallow: None,
            submodules: None,
            lfs: None,
            export_ignore: None,
            all_refs: None,
            verify_commit: None,
            keytype: None,
            public_key: None,
            public_keys: None,
            nar_hash: None,
            rev_count: None,
            last_modified: None,
            name: None,
            dirty_rev: None,
            dirty_short_rev: None,
        }
    }

    pub(crate) fn from_url(mut url: Url<'_>) -> NixUriResult<Self> {
        let mut take = |key: &str| url.query.remove(key);
        let mut git = Self::new(String::new());
        git.ref_ = take("ref").map(|r| validated_ref_name(&r)).transpose()?;
        git.rev = take("rev").map(Cow::into_owned);
        git.keytype = take("keytype").map(Cow::into_owned);
        git.public_key = take("publicKey").map(Cow::into_owned);
        git.public_keys = take("publicKeys").map(Cow::into_owned);
        git.shallow = take("shallow").map(|v| query_bool(&v));
        git.submodules = take("submodules").map(|v| query_bool(&v));
        git.lfs = take("lfs").map(|v| query_bool(&v));
        git.export_ignore = take("exportIgnore").map(|v| query_bool(&v));
        git.all_refs = take("allRefs").map(|v| query_bool(&v));
        git.verify_commit = take("verifyCommit").map(|v| query_bool(&v));
        git.url = match scp_to_ssh(&url.to_string())? {
            Some(ssh) => ssh,
            None => {
                url.strip_application();
                url.to_string()
            }
        };
        Ok(git)
    }

    pub(crate) fn from_attrs(mut attrs: AttrReader<'_>) -> NixUriResult<Self> {
        let url = attrs.required_string("url")?;
        let git = Self {
            url: fix_git_url(&url)?,
            ref_: attrs
                .string("ref")?
                .map(|r| validated_ref_name(&r))
                .transpose()?,
            rev: attrs.string("rev")?,
            shallow: attrs.bool("shallow")?,
            submodules: attrs.bool("submodules")?,
            lfs: attrs.bool("lfs")?,
            export_ignore: attrs.bool("exportIgnore")?,
            all_refs: attrs.bool("allRefs")?,
            verify_commit: attrs.bool("verifyCommit")?,
            keytype: attrs.string("keytype")?,
            public_key: attrs.string("publicKey")?,
            public_keys: attrs.string("publicKeys")?,
            nar_hash: attrs.string("narHash")?,
            rev_count: attrs.int("revCount")?,
            last_modified: attrs.int("lastModified")?,
            name: attrs.string("name")?,
            dirty_rev: attrs.string("dirtyRev")?,
            dirty_short_rev: attrs.string("dirtyShortRev")?,
        };
        attrs.finish()?;
        Ok(git)
    }

    pub(crate) fn write_attrs(&self, attrs: &mut Attrs) {
        attrs.insert("type", "git");
        attrs.insert("url", &self.url);
        attrs.set_opt("ref", self.ref_.as_ref());
        attrs.set_opt("rev", self.rev.as_ref());
        attrs.set_opt("shallow", self.shallow);
        attrs.set_opt("submodules", self.submodules);
        attrs.set_opt("lfs", self.lfs);
        attrs.set_opt("exportIgnore", self.export_ignore);
        attrs.set_opt("allRefs", self.all_refs);
        attrs.set_opt("verifyCommit", self.verify_commit);
        attrs.set_opt("keytype", self.keytype.as_ref());
        attrs.set_opt("publicKey", self.public_key.as_ref());
        attrs.set_opt("publicKeys", self.public_keys.as_ref());
        attrs.set_opt("narHash", self.nar_hash.as_ref());
        attrs.set_opt("revCount", self.rev_count);
        attrs.set_opt("lastModified", self.last_modified);
        attrs.set_opt("name", self.name.as_ref());
        attrs.set_opt("dirtyRev", self.dirty_rev.as_ref());
        attrs.set_opt("dirtyShortRev", self.dirty_short_rev.as_ref());
    }

    pub(crate) fn render(&self) -> RenderUrl<'_> {
        let prefix = |scheme: &str| if scheme == "git" { "" } else { "git+" };
        let mut out = RenderUrl::from_url_string(&self.url, prefix);
        out.query_opt("ref", self.ref_.as_deref());
        out.query_opt("rev", self.rev.as_deref().map(render_rev));
        for (key, value) in [
            ("shallow", self.shallow),
            ("lfs", self.lfs),
            ("submodules", self.submodules),
            ("exportIgnore", self.export_ignore),
            ("verifyCommit", self.verify_commit),
        ] {
            if value == Some(true) {
                out.query_opt(key, Some("1"));
            }
        }
        out.query_opt("keytype", self.keytype.as_deref());
        out.query_opt("publicKey", self.public_key.as_deref());
        out.query_opt("publicKeys", self.public_keys.as_deref());
        out
    }
}

/// Nix's normalisation of a git `url` attribute.
fn fix_git_url(url: &str) -> NixUriResult<String> {
    if url.starts_with('/') {
        let path = super::path_ref::fs_path_segments(url);
        return Ok(format!("file://{}", EncodedPath(&path)));
    }
    if let Some(ssh) = scp_to_ssh(url)? {
        return Ok(ssh);
    }
    let mut parsed = Url::parse(url)?;
    if let Some(transport) = parsed.scheme.strip_prefix("git+") {
        parsed.scheme = transport;
    }
    Ok(parsed.to_string())
}

const GIT_URL_TRANSPORTS: &[&str] = &["ssh", "http", "https", "file", "ftp", "ftps", "git"];

/// The `[user@]host` and the path of an SCP-style git location, `[user@]host:path`.
pub(crate) fn split_scp_location(url: &str) -> Option<(&str, &str)> {
    if url.contains("://") {
        return None;
    }
    let (first, _) = url.split_once(':')?;
    let transport = first.strip_prefix("git+").unwrap_or(first);
    if first.contains('/') || GIT_URL_TRANSPORTS.contains(&transport) {
        return None;
    }
    let bracket = match url.find("@[") {
        Some(at) => Some(at + 1),
        None => url.starts_with('[').then_some(0),
    };
    let host_end =
        match bracket.and_then(|start| url[start..].find(']').map(|close| start + close + 1)) {
            Some(end) if url[end..].starts_with(':') => end,
            Some(_) => return None,
            None => first.len(),
        };
    Some((&url[..host_end], &url[host_end + 1..]))
}

/// An SCP-style location as an `ssh://` URL.
fn scp_to_ssh(url: &str) -> NixUriResult<Option<String>> {
    let Some((user_host, path)) = split_scp_location(url) else {
        return Ok(None);
    };
    let invalid = || NixUriError::InvalidUrl(url.into());
    if path.is_empty() {
        return Err(invalid());
    }
    let (user, host) = match user_host.split_once('@') {
        Some((user, host)) => (Some(user), host),
        None => (None, user_host),
    };
    let host = match host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        Some(literal) => Host::Literal(
            literal
                .parse::<Ipv6Addr>()
                .map_err(|_| invalid())?
                .to_string(),
        ),
        None => Host::Name(Cow::Borrowed(host)),
    };
    let mut segments: Vec<Cow<'_, str>> = path.split('/').map(Cow::Borrowed).collect();
    if !path.starts_with('/') {
        segments.insert(0, Cow::Borrowed(""));
    }
    let ssh = Url {
        scheme: "ssh",
        authority: Some(Authority {
            user: user.map(Cow::Borrowed),
            password: None,
            host,
            port: None,
        }),
        path: segments,
        query: Default::default(),
    };
    Ok(Some(ssh.to_string()))
}
