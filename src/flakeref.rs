use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::Display;

use serde::{Deserialize, Serialize};

use crate::{
    Context,
    error::{NixUriError, UnsupportedReason},
};

pub(crate) mod attrs;
pub use attrs::{AttrValue, Attrs};
mod download;
pub(crate) use download::DOWNLOAD_TRANSPORTS;
pub use download::Download;
pub(crate) mod encoding;
mod forge;
pub use forge::{GitForge, GitForgePlatform};
pub(crate) mod fr_type;
pub(crate) mod hash;
pub use fr_type::FlakeRefType;
mod git;
pub use git::GitRepo;
pub(crate) use git::split_scp_location;
mod indirect;
pub use indirect::Indirect;
pub(crate) use indirect::is_flake_id;
mod mercurial;
pub(crate) use mercurial::HG_TRANSPORTS;
pub use mercurial::MercurialRepo;
mod path_ref;
pub use path_ref::PathRef;
pub(crate) use path_ref::nul_in_path;
#[cfg(test)]
mod proptest;
pub(crate) mod url;
pub(crate) mod validators;

/// A flake reference.
///
/// The fields are Nix's attributes ([`Self::to_attrs`] is `builtins.parseFlakeRef`), and
/// [`Display`] is `builtins.flakeRefToString`. Parsing normalises: `github:o/r?ref=main`
/// renders as `github:o/r/main`, and `nixpkgs` as `flake:nixpkgs`.
///
/// Serialises as its attribute set, the JSON of a `flake.lock` node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "Attrs", try_from = "Attrs")]
#[non_exhaustive]
pub struct FlakeRef {
    /// What the reference points to. Reachable through [`Self::kind`] and
    /// [`Self::kind_mut`].
    pub(crate) kind: FlakeRefType,
    dir: Option<String>,
}

/// Identity of a git-forge flake ref: `(platform, owner, repo, domain)`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ForgeIdentity {
    pub platform: GitForgePlatform,
    pub owner: String,
    pub repo: String,
    pub domain: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RefKind {
    None,
    Ref,
    Rev,
    Both,
}

#[derive(Debug)]
pub(crate) struct RenderUrl<'a> {
    base: RenderBase<'a>,
    query: BTreeMap<Cow<'a, str>, Cow<'a, str>>,
}

#[derive(Debug)]
enum RenderBase<'a> {
    /// `<scheme>:` and a path given as decoded segments.
    Segments {
        scheme: &'static str,
        segments: Vec<Cow<'a, str>>,
    },
    Url {
        prefix: &'static str,
        url: url::Url<'a>,
    },
    Verbatim {
        prefix: &'static str,
        url: &'a str,
    },
}

impl<'a> RenderUrl<'a> {
    pub(crate) fn from_segments(scheme: &'static str, segments: Vec<Cow<'a, str>>) -> Self {
        Self {
            base: RenderBase::Segments { scheme, segments },
            query: BTreeMap::new(),
        }
    }

    pub(crate) fn from_url_string(url: &'a str, prefix: impl Fn(&str) -> &'static str) -> Self {
        match url::Url::parse(url) {
            Ok(mut parsed) => {
                let query = std::mem::take(&mut parsed.query);
                Self {
                    base: RenderBase::Url {
                        prefix: prefix(parsed.scheme),
                        url: parsed,
                    },
                    query,
                }
            }
            Err(_) => Self {
                base: RenderBase::Verbatim {
                    prefix: prefix(url::scheme_of(url).unwrap_or_default()),
                    url,
                },
                query: BTreeMap::new(),
            },
        }
    }

    pub(crate) fn query_opt(&mut self, key: &'a str, value: Option<impl Into<Cow<'a, str>>>) {
        if let Some(value) = value {
            self.query.insert(Cow::Borrowed(key), value.into());
        }
    }
}

impl Display for RenderUrl<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.base {
            RenderBase::Segments { scheme, segments } => {
                write!(f, "{scheme}:{}", url::EncodedPath(segments))?;
            }
            RenderBase::Url { prefix, url } => write!(f, "{prefix}{url}")?,
            RenderBase::Verbatim { prefix, url } => write!(f, "{prefix}{url}")?,
        }
        write!(f, "{}", url::EncodedQuery(&self.query))
    }
}

impl FlakeRef {
    /// Parse `input` as a flake reference in `context`, as `builtins.parseFlakeRef`
    /// ([`Context::Standalone`]) or a flake input ([`Context::FlakeInput`]) does.
    ///
    /// ```
    /// # use nix_uri::{Context, FlakeRef, NixUriError};
    /// let flake_ref = FlakeRef::parse("github:NixOS/nixpkgs?ref=nixos-25.05", Context::Standalone)?;
    /// assert_eq!(flake_ref.to_string(), "github:NixOS/nixpkgs/nixos-25.05");
    ///
    /// // Only a flake input keeps a relative path.
    /// assert!(FlakeRef::parse("./foo", Context::Standalone).is_err());
    /// assert_eq!(FlakeRef::parse("./foo", Context::FlakeInput)?.to_string(), "path:./foo");
    /// # Ok::<(), NixUriError>(())
    /// ```
    pub fn parse(input: &str, context: Context) -> Result<Self, NixUriError> {
        let (flake_ref, fragment) = crate::parser::parse_nix_uri(input, context)?;
        match fragment {
            Some(fragment) => Err(NixUriError::Unsupported(UnsupportedReason::Fragment {
                fragment,
            })),
            None => Ok(flake_ref),
        }
    }

    /// Parse `<flake reference>#<attribute path>`, Nix's `parseFlakeRefWithFragment`: the
    /// rules of [`Context::FlakeInput`], plus a percent-decoded fragment. An empty
    /// fragment is `None`.
    ///
    /// ```
    /// # use nix_uri::{FlakeRef, NixUriError};
    /// let (flake_ref, fragment) = FlakeRef::parse_with_fragment("nixpkgs#hello")?;
    /// assert_eq!(flake_ref.to_string(), "flake:nixpkgs");
    /// assert_eq!(fragment.as_deref(), Some("hello"));
    /// # Ok::<(), NixUriError>(())
    /// ```
    pub fn parse_with_fragment(input: &str) -> Result<(Self, Option<String>), NixUriError> {
        crate::parser::parse_nix_uri(input, Context::FlakeInput)
    }

    /// Build a flake reference from its attributes: a node of `flake.lock`, a registry
    /// entry, or the result of `builtins.parseFlakeRef`.
    ///
    /// ```
    /// # use nix_uri::{Attrs, FlakeRef, NixUriError};
    /// let attrs: Attrs = serde_json::from_str(
    ///     r#"{"type": "github", "owner": "NixOS", "repo": "nixpkgs", "lastModified": 1700000000}"#,
    /// ).unwrap();
    /// let flake_ref = FlakeRef::from_attrs(&attrs)?;
    /// assert_eq!(flake_ref.to_string(), "github:NixOS/nixpkgs");
    /// assert_eq!(flake_ref.to_attrs(), attrs);
    /// # Ok::<(), NixUriError>(())
    /// ```
    pub fn from_attrs(attrs: &Attrs) -> Result<Self, NixUriError> {
        let ty = match attrs.get("type") {
            Some(AttrValue::String(ty)) => ty.as_str(),
            Some(_) => {
                return Err(NixUriError::InvalidValue {
                    field: "type",
                    reason: "expected a string".to_string(),
                });
            }
            None => {
                return Err(NixUriError::Unsupported(
                    UnsupportedReason::MissingParameter {
                        ty: "flake reference".to_string(),
                        parameter: "type".to_string(),
                    },
                ));
            }
        };
        let forge = GitForgePlatform::from_type(ty);
        let kind = match ty {
            _ if let Some(platform) = forge => FlakeRefType::GitForge(GitForge::from_attrs(
                platform,
                attrs::AttrReader::new(platform.as_str(), attrs),
            )?),
            "git" => FlakeRefType::Git(GitRepo::from_attrs(attrs::AttrReader::new("git", attrs))?),
            "hg" => FlakeRefType::Mercurial(MercurialRepo::from_attrs(attrs::AttrReader::new(
                "hg", attrs,
            ))?),
            "tarball" => FlakeRefType::Tarball(Download::from_attrs(attrs::AttrReader::new(
                "tarball", attrs,
            ))?),
            "file" => {
                FlakeRefType::File(Download::from_attrs(attrs::AttrReader::new("file", attrs))?)
            }
            "path" => {
                FlakeRefType::Path(PathRef::from_attrs(attrs::AttrReader::new("path", attrs))?)
            }
            "indirect" => FlakeRefType::Indirect(Indirect::from_attrs(attrs::AttrReader::new(
                "indirect", attrs,
            ))?),
            _ => {
                return Err(NixUriError::Unsupported(UnsupportedReason::UriType {
                    ty: ty.to_string(),
                }));
            }
        };
        let dir = match attrs.get("dir") {
            None => None,
            Some(AttrValue::String(dir)) => Some(dir.clone()),
            Some(_) => {
                return Err(NixUriError::InvalidValue {
                    field: "dir",
                    reason: "expected a string".to_string(),
                });
            }
        };
        Ok(Self::new(kind).with_dir(dir))
    }

    #[must_use]
    pub fn to_attrs(&self) -> Attrs {
        let mut attrs = Attrs::new();
        match &self.kind {
            FlakeRefType::GitForge(forge) => forge.write_attrs(&mut attrs),
            FlakeRefType::Git(git) => git.write_attrs(&mut attrs),
            FlakeRefType::Mercurial(hg) => hg.write_attrs(&mut attrs),
            FlakeRefType::Tarball(download) => download.write_attrs("tarball", &mut attrs),
            FlakeRefType::File(download) => download.write_attrs("file", &mut attrs),
            FlakeRefType::Path(path) => path.write_attrs(&mut attrs),
            FlakeRefType::Indirect(indirect) => indirect.write_attrs(&mut attrs),
        }
        attrs.set_opt("dir", self.dir.as_ref());
        attrs
    }

    /// A flake reference to `kind` at the root of its source tree.
    #[must_use]
    pub fn new(kind: FlakeRefType) -> Self {
        Self { kind, dir: None }
    }

    #[must_use]
    pub fn kind(&self) -> &FlakeRefType {
        &self.kind
    }

    /// Mutable access to `kind`, to edit its fields in place.
    pub fn kind_mut(&mut self) -> &mut FlakeRefType {
        &mut self.kind
    }

    /// The subdirectory of the source tree that holds `flake.nix`.
    #[must_use]
    pub fn dir(&self) -> Option<&str> {
        self.dir.as_deref()
    }

    /// The repository name of a git forge, or the second path segment of a git URL
    /// (`host/owner/<repo>`, without `.git`).
    #[must_use]
    pub fn repo(&self) -> Option<String> {
        self.kind.repo()
    }

    /// The owner of a git forge, or the first path segment of a git URL.
    #[must_use]
    pub fn owner(&self) -> Option<String> {
        self.kind.owner()
    }

    #[must_use]
    pub fn domain(&self) -> Option<String> {
        self.kind.domain()
    }

    /// Platform, owner, repository and host of a git-forge reference.
    #[must_use]
    pub fn forge_identity(&self) -> Option<ForgeIdentity> {
        match &self.kind {
            FlakeRefType::GitForge(forge) => Some(ForgeIdentity {
                platform: forge.platform,
                owner: forge.owner.clone(),
                repo: forge.repo.clone(),
                domain: self.kind.domain()?,
            }),
            _ => None,
        }
    }

    /// The branch or tag name.
    #[must_use]
    pub fn ref_(&self) -> Option<&str> {
        self.kind.ref_()
    }

    /// The revision, as given.
    #[must_use]
    pub fn rev(&self) -> Option<&str> {
        self.kind.rev()
    }

    /// The revision if there is one, the ref otherwise.
    #[must_use]
    pub fn ref_or_rev(&self) -> Option<&str> {
        self.rev().or_else(|| self.ref_())
    }

    #[must_use]
    pub fn ref_kind(&self) -> RefKind {
        match (self.ref_().is_some(), self.rev().is_some()) {
            (false, false) => RefKind::None,
            (true, false) => RefKind::Ref,
            (false, true) => RefKind::Rev,
            (true, true) => RefKind::Both,
        }
    }

    #[must_use]
    pub fn is_pinned_to_rev(&self) -> bool {
        self.rev().is_some()
    }

    /// Set or clear the branch or tag name.
    pub fn with_ref(mut self, new_ref: Option<String>) -> Result<Self, NixUriError> {
        let Some(new_ref) = new_ref else {
            if let Some(slot) = self.kind.ref_slot() {
                *slot = None;
            }
            return Ok(self);
        };
        let new_ref = match &self.kind {
            FlakeRefType::Mercurial(_) if !validators::matches_ref_shape(&new_ref) => {
                return Err(NixUriError::InvalidValue {
                    field: "ref",
                    reason: "expected a Mercurial branch or tag name".to_string(),
                });
            }
            FlakeRefType::Mercurial(_) => new_ref,
            _ => validators::validated_ref_name(&new_ref)?,
        };
        if matches!(
            &self.kind,
            FlakeRefType::GitForge(GitForge { rev: Some(_), .. })
        ) {
            return Err(NixUriError::FieldConflict {
                left: "ref",
                right: "rev",
            });
        }
        let Some(slot) = self.kind.ref_slot() else {
            return Err(NixUriError::Unsupported(UnsupportedReason::Field {
                field: "ref".into(),
                only_supported_by: "github, gitlab, sourcehut, git, hg, indirect".into(),
            }));
        };
        *slot = Some(new_ref);
        Ok(self)
    }

    #[must_use]
    pub fn pin_to_rev(mut self, rev: String) -> Self {
        if let FlakeRefType::GitForge(forge) = &mut self.kind {
            forge.ref_ = None;
        }
        *self.kind.rev_slot() = Some(rev);
        self
    }

    pub fn with_rev(mut self, rev: Option<String>) -> Result<Self, NixUriError> {
        if rev.is_some()
            && matches!(
                &self.kind,
                FlakeRefType::GitForge(GitForge { ref_: Some(_), .. })
            )
        {
            return Err(NixUriError::FieldConflict {
                left: "ref",
                right: "rev",
            });
        }
        *self.kind.rev_slot() = rev;
        Ok(self)
    }

    /// Set or clear the subdirectory.
    #[must_use]
    pub fn with_dir(mut self, dir: Option<String>) -> Self {
        self.dir = dir.filter(|dir| !dir.is_empty());
        self
    }
}

/// Mirrors `builtins.flakeRefToString`.
impl Display for FlakeRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut url = match &self.kind {
            FlakeRefType::GitForge(forge) => forge.render(),
            FlakeRefType::Git(git) => git.render(),
            FlakeRefType::Mercurial(hg) => hg.render(),
            FlakeRefType::Tarball(download) | FlakeRefType::File(download) => download.render(),
            FlakeRefType::Path(path) => path.render(),
            FlakeRefType::Indirect(indirect) => indirect.render(),
        };
        url.query_opt("dir", self.dir.as_deref());
        url.fmt(f)
    }
}

impl From<FlakeRef> for Attrs {
    fn from(flake_ref: FlakeRef) -> Self {
        flake_ref.to_attrs()
    }
}

impl TryFrom<Attrs> for FlakeRef {
    type Error = NixUriError;

    fn try_from(attrs: Attrs) -> Result<Self, Self::Error> {
        Self::from_attrs(&attrs)
    }
}

impl TryFrom<&str> for FlakeRef {
    type Error = NixUriError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value, Context::default())
    }
}

impl std::str::FromStr for FlakeRef {
    type Err = NixUriError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s, Context::default())
    }
}

#[cfg(test)]
mod from_attrs {
    //! `builtins.flakeRefToString` of an attribute set, in Nix 2.35.3.
    use std::assert_matches;

    use rstest::rstest;

    use crate::{Attrs, FlakeRef, NixUriError, UnsupportedReason};

    fn attrs(json: &str) -> Attrs {
        serde_json::from_str(json).unwrap()
    }

    #[rstest]
    #[case(
        "{\"dir\":\"lib\",\"lastModified\":1700000000,\"narHash\":\"sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\",\"owner\":\"NixOS\",\"repo\":\"nixpkgs\",\"rev\":\"a3a3dda3bacf61e8a39258a0ed9c924eeca8e293\",\"type\":\"github\"}",
        "github:NixOS/nixpkgs/a3a3dda3bacf61e8a39258a0ed9c924eeca8e293?dir=lib&narHash=sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA%3D"
    )]
    #[case(
        "{\"lastModified\":1,\"ref\":\"main\",\"rev\":\"a3a3dda3bacf61e8a39258a0ed9c924eeca8e293\",\"revCount\":5,\"submodules\":true,\"type\":\"git\",\"url\":\"https://h.org/r\"}",
        "git+https://h.org/r?ref=main&rev=a3a3dda3bacf61e8a39258a0ed9c924eeca8e293&submodules=1"
    )]
    #[case(
        "{\"lastModified\":1,\"narHash\":\"sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\",\"path\":\"/nix/store/abc-source\",\"type\":\"path\"}",
        "path:/nix/store/abc-source?lastModified=1&narHash=sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA%3D"
    )]
    #[case(
        "{\"lastModified\":1,\"owner\":\"o\",\"repo\":\"r\",\"treeHash\":\"abc\",\"type\":\"github\"}",
        "github:o/r"
    )]
    #[case(
        "{\"id\":\"nixpkgs\",\"ref\":\"nixos-25.05\",\"type\":\"indirect\"}",
        "flake:nixpkgs/nixos-25.05"
    )]
    #[case(
        "{\"path\":\"github:NixOS/nixpkgs\",\"type\":\"path\"}",
        "path:github:NixOS/nixpkgs"
    )]
    #[case(
        "{\"__final\":true,\"allRefs\":true,\"dirtyRev\":\"x-dirty\",\"dirtyShortRev\":\"x-dirty\",\"exportIgnore\":true,\"lfs\":true,\"name\":\"n\",\"narHash\":\"sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\",\"shallow\":false,\"type\":\"git\",\"url\":\"https://h.org/r\"}",
        "git+https://h.org/r?exportIgnore=1&lfs=1"
    )]
    #[case(
        "{\"type\":\"git\",\"url\":\"git@github.com:o/r\"}",
        "git+ssh://git@github.com/o/r"
    )]
    #[case("{\"type\":\"git\",\"url\":\"/local/x\"}", "git+file:///local/x")]
    #[case(
        "{\"rev\":\"A3A3DDA3BACF61E8A39258A0ED9C924EECA8E293\",\"type\":\"git\",\"url\":\"https://h/r?b=1&a=2\"}",
        "git+https://h/r?a=2&b=1&rev=a3a3dda3bacf61e8a39258a0ed9c924eeca8e293"
    )]
    #[case(
        "{\"name\":\"x\",\"narHash\":\"sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\",\"ref\":\"default\",\"rev\":\"a3a3dda3bacf61e8a39258a0ed9c924eeca8e293\",\"revCount\":1,\"type\":\"hg\",\"url\":\"https://h/r\"}",
        "hg+https://h/r?ref=default&rev=a3a3dda3bacf61e8a39258a0ed9c924eeca8e293"
    )]
    #[case(
        "{\"lastModified\":2,\"name\":\"n\",\"narHash\":\"sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\",\"rev\":\"a3a3dda3bacf61e8a39258a0ed9c924eeca8e293\",\"revCount\":1,\"type\":\"tarball\",\"unpack\":true,\"url\":\"https://h/x\"}",
        "https://h/x?narHash=sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA%3D"
    )]
    #[case("{\"type\":\"file\",\"url\":\"https://h/x.json\"}", "https://h/x.json")]
    #[case("{\"type\":\"file\",\"url\":\"file:./x\"}", "file:./x")]
    #[case(
        "{\"id\":\"nixpkgs\",\"narHash\":\"sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\",\"ref\":\"a/b\",\"rev\":\"a3a3dda3bacf61e8a39258a0ed9c924eeca8e293\",\"type\":\"indirect\"}",
        "flake:nixpkgs/a%2Fb/a3a3dda3bacf61e8a39258a0ed9c924eeca8e293"
    )]
    #[case("{\"path\":\"/a/b/\",\"type\":\"path\"}", "path:/a/b//")]
    #[case("{\"path\":\"\",\"type\":\"path\"}", "path:")]
    #[case("{\"path\":\"./a b\",\"type\":\"path\"}", "path:./a%20b")]
    #[case(
        "{\"host\":\"git.example.org\",\"owner\":\"o/sub\",\"ref\":\"feature/x\",\"repo\":\"r\",\"type\":\"gitlab\"}",
        "gitlab:o%2Fsub/r/feature%2Fx?host=git.example.org"
    )]
    fn renders_like_flake_ref_to_string(#[case] json: &str, #[case] display: &str) {
        let flake_ref = FlakeRef::from_attrs(&attrs(json)).unwrap();
        assert_eq!(flake_ref.to_string(), display);
    }

    #[test]
    fn keeps_the_attributes() {
        let json = r#"{"dir":"lib","lastModified":1700000000,"narHash":"sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=","owner":"NixOS","repo":"nixpkgs","rev":"a3a3dda3bacf61e8a39258a0ed9c924eeca8e293","type":"github"}"#;
        assert_eq!(
            FlakeRef::from_attrs(&attrs(json)).unwrap().to_attrs(),
            attrs(json)
        );
    }

    #[test]
    fn empty_dir_is_no_dir() {
        let flake_ref =
            FlakeRef::from_attrs(&attrs(r#"{"type":"path","path":"/x","dir":""}"#)).unwrap();
        assert_eq!(flake_ref.dir(), None);
        assert_eq!(flake_ref.to_string(), "path:/x");
    }

    #[test]
    fn final_is_ignored() {
        let flake_ref =
            FlakeRef::from_attrs(&attrs(r#"{"type":"path","path":"/x","__final":true}"#)).unwrap();
        assert_eq!(
            flake_ref.to_attrs(),
            attrs(r#"{"type":"path","path":"/x"}"#)
        );
    }

    /// The forms Nix normalises a git `url` attribute into.
    #[rstest]
    #[case("git@github.com:o/r", "ssh://git@github.com/o/r")]
    #[case("github.com:o/r", "ssh://github.com/o/r")]
    #[case("git+github:o/r", "ssh://git%2Bgithub/o/r")]
    #[case("https:o/r", "https:o/r")]
    #[case("git+ftps:x", "ftps:x")]
    #[case("a@b@c:x", "ssh://a@b%40c/x")]
    #[case("h%20:x", "ssh://h%2520/x")]
    #[case("h:o/r?x=1", "ssh://h/o/r%3Fx%3D1")]
    #[case(":x", "ssh:///x")]
    #[case("@h:x", "ssh://@h/x")]
    #[case("u@[::1]:x", "ssh://u@[::1]/x")]
    #[case("git+foo:a@[::1]:x", "ssh://git%2Bfoo%3Aa@[::1]/x")]
    #[case("[0:0:0:0:0:0:0:1]:x", "ssh://[::1]/x")]
    #[case("[::FFFF:1.2.3.4]:x", "ssh://[::ffff:1.2.3.4]/x")]
    #[case("[::1.2.3.4]:x", "ssh://[::102:304]/x")]
    #[case("[1:0:0:0:1:0:0:1]:x", "ssh://[1::1:0:0:1]/x")]
    #[case("/local/x", "file:///local/x")]
    #[case("git+https://h/a*b", "https://h/a%2Ab")]
    fn git_url_is_normalised(#[case] url: &str, #[case] normalised: &str) {
        let json = format!(r#"{{"type":"git","url":"{url}"}}"#);
        let flake_ref = FlakeRef::from_attrs(&attrs(&json)).unwrap();
        assert_eq!(
            flake_ref.to_attrs(),
            attrs(&format!(r#"{{"type":"git","url":"{normalised}"}}"#))
        );
    }

    #[rstest]
    #[case("h:")]
    #[case("u@[::1]:")]
    #[case("[fe80::1%25eth0]:x")]
    #[case("[v1.x]:x")]
    #[case("[zz]:x")]
    fn malformed_scp_git_url(#[case] url: &str) {
        let json = format!(r#"{{"type":"git","url":"{url}"}}"#);
        assert_matches!(
            FlakeRef::from_attrs(&attrs(&json)),
            Err(NixUriError::InvalidUrl(_))
        );
    }

    /// Each type takes its own attributes. Rejections checked against
    /// `builtins.flakeRefToString` in Nix 2.35.3.
    #[rstest]
    #[case(r#"{"type":"github","owner":"o","repo":"r","name":"x"}"#)]
    #[case(r#"{"type":"github","owner":"o","repo":"r","extra":1}"#)]
    #[case(r#"{"type":"git","url":"https://h.org/r","treeHash":"abc"}"#)]
    #[case(r#"{"type":"git","url":"https://h.org/r","host":"x"}"#)]
    fn unsupported_attribute(#[case] json: &str) {
        assert_matches!(
            FlakeRef::from_attrs(&attrs(json)),
            Err(NixUriError::Unsupported(UnsupportedReason::Param { .. }))
        );
    }

    #[test]
    fn wrong_attribute_type() {
        assert_matches!(
            FlakeRef::from_attrs(&attrs(
                r#"{"type":"github","owner":"o","repo":"r","lastModified":"1"}"#
            )),
            Err(NixUriError::InvalidValue {
                field: "lastModified",
                ..
            })
        );
    }

    #[test]
    fn forge_takes_a_ref_or_a_rev() {
        assert_matches!(
            FlakeRef::from_attrs(&attrs(
                r#"{"type":"github","owner":"o","repo":"r","ref":"main","rev":"a3a3dda3bacf61e8a39258a0ed9c924eeca8e293"}"#
            )),
            Err(NixUriError::FieldConflict { .. })
        );
        assert_matches!(
            FlakeRef::from_attrs(&attrs(
                r#"{"type":"github","owner":"o","repo":"r","ref":"a b"}"#
            )),
            Err(NixUriError::InvalidValue { field: "ref", .. })
        );
    }

    #[test]
    fn missing_or_unknown_type() {
        assert_matches!(
            FlakeRef::from_attrs(&attrs(r#"{"owner":"o"}"#)),
            Err(NixUriError::Unsupported(
                UnsupportedReason::MissingParameter { .. }
            ))
        );
        assert_matches!(
            FlakeRef::from_attrs(&attrs(r#"{"type":"mercurial","url":"https://h/r"}"#)),
            Err(NixUriError::Unsupported(UnsupportedReason::UriType { .. }))
        );
        assert_matches!(
            FlakeRef::from_attrs(&attrs(r#"{"type":"github","owner":"o"}"#)),
            Err(NixUriError::Unsupported(
                UnsupportedReason::MissingParameter { .. }
            ))
        );
    }

    /// `FlakeRef` serialises as its attributes.
    #[test]
    fn serde_is_the_lock_file_shape() {
        let flake_ref: FlakeRef = "github:NixOS/nixpkgs/nixos-25.05?dir=lib".parse().unwrap();
        let json = serde_json::to_string(&flake_ref).unwrap();
        assert_eq!(
            json,
            r#"{"dir":"lib","owner":"NixOS","ref":"nixos-25.05","repo":"nixpkgs","type":"github"}"#
        );
        assert_eq!(serde_json::from_str::<FlakeRef>(&json).unwrap(), flake_ref);
    }
}

#[cfg(test)]
mod builders {
    use std::assert_matches;

    use rstest::rstest;

    use crate::{Context, FlakeRef, NixUriError, RefKind, UnsupportedReason};

    fn parse(s: &str) -> FlakeRef {
        FlakeRef::parse(s, Context::FlakeInput).unwrap()
    }

    const REV: &str = "a3a3dda3bacf61e8a39258a0ed9c924eeca8e293";

    #[test]
    fn with_ref() {
        let flake_ref = parse("github:o/r").with_ref(Some("main".into())).unwrap();
        assert_eq!(flake_ref.to_string(), "github:o/r/main");
        assert_eq!(flake_ref.ref_kind(), RefKind::Ref);
        let flake_ref = flake_ref.with_ref(None).unwrap();
        assert_eq!(flake_ref.to_string(), "github:o/r");
    }

    #[test]
    fn with_ref_checks_the_name() {
        assert_matches!(
            parse("github:o/r").with_ref(Some("a..b".into())),
            Err(NixUriError::InvalidValue { field: "ref", .. })
        );
        assert_matches!(
            parse("hg+https://h/r").with_ref(Some("-x".into())),
            Err(NixUriError::InvalidValue { field: "ref", .. })
        );
    }

    #[rstest]
    #[case("https://h/x.tar.gz")]
    #[case("/foo")]
    #[case("file+https://h/x")]
    fn with_ref_on_a_kind_without_one(#[case] input: &str) {
        assert_matches!(
            parse(input).with_ref(Some("main".into())),
            Err(NixUriError::Unsupported(UnsupportedReason::Field { .. }))
        );
    }

    #[test]
    fn forge_takes_a_ref_or_a_rev() {
        assert_matches!(
            parse(&format!("github:o/r/{REV}")).with_ref(Some("main".into())),
            Err(NixUriError::FieldConflict { .. })
        );
        assert_matches!(
            parse("github:o/r/main").with_rev(Some(REV.into())),
            Err(NixUriError::FieldConflict { .. })
        );
        let pinned = parse("github:o/r/main").pin_to_rev(REV.into());
        assert_eq!(pinned.to_string(), format!("github:o/r/{REV}"));
    }

    #[test]
    fn git_keeps_the_ref_when_pinned() {
        let pinned = parse("git+https://h/r?ref=main").pin_to_rev(REV.into());
        assert_eq!(
            pinned.to_string(),
            format!("git+https://h/r?ref=main&rev={REV}")
        );
        assert_eq!(pinned.ref_kind(), RefKind::Both);
        let unpinned = pinned.with_rev(None).unwrap();
        assert_eq!(unpinned.to_string(), "git+https://h/r?ref=main");
    }

    #[test]
    fn with_dir() {
        let flake_ref = parse("path:/x").with_dir(Some("sub dir".into()));
        assert_eq!(flake_ref.dir(), Some("sub dir"));
        assert_eq!(flake_ref.to_string(), "path:/x?dir=sub%20dir");
    }

    #[test]
    fn with_empty_dir() {
        let flake_ref = parse("path:/x?dir=a").with_dir(Some(String::new()));
        assert_eq!(flake_ref.dir(), None);
        assert_eq!(flake_ref.to_string(), "path:/x");
    }

    #[test]
    fn forge_identity_uses_the_host() {
        let identity = parse("gitlab:o/r?host=git.example.org")
            .forge_identity()
            .unwrap();
        assert_eq!(identity.domain, "git.example.org");
        assert_eq!(
            parse("sourcehut:~o/r").forge_identity().unwrap().domain,
            "git.sr.ht"
        );
        assert!(parse("git+https://h/o/r").forge_identity().is_none());
    }
}
