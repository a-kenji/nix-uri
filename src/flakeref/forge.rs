use std::borrow::Cow;
use std::fmt::Display;

use crate::{
    error::{NixUriError, NixUriResult},
    flakeref::{
        RenderUrl,
        attrs::{AttrReader, Attrs},
        hash::{render_nar_hash, rev_to_base16},
        validators::validated_ref_name,
    },
};

/// Which git-forge scheme a `GitForge` reference uses: the Nix type `github`, `gitlab` or
/// `sourcehut`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GitForgePlatform {
    GitHub,
    GitLab,
    SourceHut,
}

impl GitForgePlatform {
    pub(crate) fn from_type(ty: &str) -> Option<Self> {
        match ty {
            "github" => Some(Self::GitHub),
            "gitlab" => Some(Self::GitLab),
            "sourcehut" => Some(Self::SourceHut),
            _ => None,
        }
    }

    /// The Nix type name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GitHub => "github",
            Self::GitLab => "gitlab",
            Self::SourceHut => "sourcehut",
        }
    }

    /// The host Nix fetches from when no `host` is given.
    pub fn default_host(self) -> &'static str {
        match self {
            Self::GitHub => "github.com",
            Self::GitLab => "gitlab.com",
            Self::SourceHut => "git.sr.ht",
        }
    }
}

impl Display for GitForgePlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A repository on a git forge (`github:`, `gitlab:`, `sourcehut:`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GitForge {
    pub platform: GitForgePlatform,
    pub owner: String,
    pub repo: String,
    pub ref_: Option<String>,
    pub rev: Option<String>,
    /// A self-hosted instance, such as `git.example.org` for GitLab.
    pub host: Option<String>,
    pub nar_hash: Option<String>,
    pub last_modified: Option<u64>,
    pub tree_hash: Option<String>,
}

impl GitForge {
    pub fn new(platform: GitForgePlatform, owner: String, repo: String) -> Self {
        Self {
            platform,
            owner,
            repo,
            ref_: None,
            rev: None,
            host: None,
            nar_hash: None,
            last_modified: None,
            tree_hash: None,
        }
    }

    pub(crate) fn from_attrs(
        platform: GitForgePlatform,
        mut attrs: AttrReader<'_>,
    ) -> NixUriResult<Self> {
        let forge = Self {
            platform,
            owner: attrs.required_string("owner")?,
            repo: attrs.required_string("repo")?,
            ref_: attrs
                .string("ref")?
                .map(|r| validated_ref_name(&r))
                .transpose()?,
            rev: attrs.string("rev")?,
            host: attrs.string("host")?,
            nar_hash: attrs.string("narHash")?,
            last_modified: attrs.int("lastModified")?,
            tree_hash: attrs.string("treeHash")?,
        };
        attrs.finish()?;
        if forge.ref_.is_some() && forge.rev.is_some() {
            return Err(NixUriError::FieldConflict {
                left: "ref",
                right: "rev",
            });
        }
        Ok(forge)
    }

    pub(crate) fn write_attrs(&self, attrs: &mut Attrs) {
        attrs.insert("type", self.platform.as_str());
        attrs.insert("owner", &self.owner);
        attrs.insert("repo", &self.repo);
        attrs.set_opt("ref", self.ref_.as_ref());
        attrs.set_opt("rev", self.rev.as_ref());
        attrs.set_opt("host", self.host.as_ref());
        attrs.set_opt("narHash", self.nar_hash.as_ref());
        attrs.set_opt("lastModified", self.last_modified);
        attrs.set_opt("treeHash", self.tree_hash.as_ref());
    }

    /// `<type>:<owner>/<repo>[/<ref or rev>]?host=&narHash=`.
    pub(crate) fn render(&self) -> RenderUrl<'_> {
        let mut segments = vec![Cow::from(&self.owner), Cow::from(&self.repo)];
        match (&self.ref_, &self.rev) {
            (Some(ref_), _) => segments.push(Cow::from(ref_)),
            (None, Some(rev)) => segments.push(Cow::from(render_rev(rev))),
            (None, None) => {}
        }
        let mut url = RenderUrl::from_segments(self.platform.as_str(), segments);
        url.query_opt("host", self.host.as_deref());
        url.query_opt("narHash", self.nar_hash.as_deref().map(render_nar_hash));
        url
    }
}

pub(crate) fn render_rev(rev: &str) -> String {
    rev_to_base16(rev).unwrap_or_else(|| rev.to_string())
}

#[cfg(test)]
mod owner_repo {
    //! Git-forge owners and repos are not validated beyond URL syntax, and each segment
    //! is percent-decoded after splitting on `/`. The expected `Display` is
    //! `builtins.flakeRefToString` in Nix 2.35.3.
    use rstest::rstest;
    use std::assert_matches;

    use crate::{Context, FlakeRef, FlakeRefType, GitForge, NixUriError, UnsupportedReason};

    fn forge(input: &str) -> GitForge {
        match FlakeRef::parse(input, Context::Standalone) {
            Ok(parsed) => match parsed.kind {
                FlakeRefType::GitForge(forge) => forge,
                other => panic!("{input:?}: expected GitForge, got {other:?}"),
            },
            Err(err) => panic!("{input:?}: expected accept, got {err:?}"),
        }
    }

    #[rstest]
    #[case("github:nixos/nixpkgs", "nixos", "nixpkgs", "github:nixos/nixpkgs")]
    #[case(
        "github:nix.os/nix-pkgs",
        "nix.os",
        "nix-pkgs",
        "github:nix.os/nix-pkgs"
    )]
    #[case("github:.dotted/repo", ".dotted", "repo", "github:.dotted/repo")]
    #[case("github:owner/.dotrepo", "owner", ".dotrepo", "github:owner/.dotrepo")]
    #[case("github:-dashed/repo", "-dashed", "repo", "github:-dashed/repo")]
    #[case(
        "github:bad!owner/repo",
        "bad!owner",
        "repo",
        "github:bad%21owner/repo"
    )]
    #[case("github:~tilde/repo", "~tilde", "repo", "github:~tilde/repo")]
    #[case("gitlab:~owner/repo", "~owner", "repo", "gitlab:~owner/repo")]
    #[case(
        "sourcehut:~misterio/nix-colors",
        "~misterio",
        "nix-colors",
        "sourcehut:~misterio/nix-colors"
    )]
    #[case(
        "sourcehut:nixos/nixpkgs",
        "nixos",
        "nixpkgs",
        "sourcehut:nixos/nixpkgs"
    )]
    #[case(
        "gitlab:veloren%2Fdev/rfcs",
        "veloren/dev",
        "rfcs",
        "gitlab:veloren%2Fdev/rfcs"
    )]
    #[case("gitlab:o%2Fp%2Fq/r", "o/p/q", "r", "gitlab:o%2Fp%2Fq/r")]
    #[case("gitlab:%2Fp/r", "/p", "r", "gitlab:%2Fp/r")]
    #[case("gitlab:p%2F/r", "p/", "r", "gitlab:p%2F/r")]
    #[case("gitlab:p%2F%2Fq/r", "p//q", "r", "gitlab:p%2F%2Fq/r")]
    #[case("github:o%2Fp/r", "o/p", "r", "github:o%2Fp/r")]
    #[case("sourcehut:o%2Fp/r", "o/p", "r", "sourcehut:o%2Fp/r")]
    #[case("github:o/r%2Fx", "o", "r/x", "github:o/r%2Fx")]
    #[case("github:o/r%20x", "o", "r x", "github:o/r%20x")]
    fn accepted(
        #[case] input: &str,
        #[case] owner: &str,
        #[case] repo: &str,
        #[case] display: &str,
    ) {
        let parsed = forge(input);
        assert_eq!(parsed.owner, owner, "{input:?}");
        assert_eq!(parsed.repo, repo, "{input:?}");
        assert_eq!(
            FlakeRef::parse(input, Context::Standalone)
                .unwrap()
                .to_string(),
            display
        );
    }

    #[test]
    fn subgroup_with_ref_query() {
        let parsed = forge("gitlab:o%2Fp/r?ref=main");
        assert_eq!(parsed.owner, "o/p");
        assert_eq!(parsed.ref_.as_deref(), Some("main"));
    }

    #[test]
    fn subgroup_with_path_rev() {
        let rev = "0123456789abcdef0123456789abcdef01234567";
        let parsed = forge(&format!("gitlab:o%2Fp/r/{rev}"));
        assert_eq!(parsed.owner, "o/p");
        assert_eq!(parsed.rev.as_deref(), Some(rev));
        assert_eq!(parsed.ref_, None);
    }

    #[test]
    fn encoded_slash_in_ref_is_part_of_the_ref() {
        assert_eq!(forge("github:o/r/a%2Fb").ref_.as_deref(), Some("a/b"));
    }

    #[test]
    fn missing_repo() {
        assert_matches!(
            FlakeRef::parse("github:owner", Context::Standalone),
            Err(NixUriError::Unsupported(UnsupportedReason::MissingParameter { parameter, .. }))
                if parameter == "repo"
        );
    }

    #[rstest]
    #[case("github:bad owner/repo")]
    #[case("github:owner/bad repo")]
    #[case("github:o/r[x")]
    #[case("github:o/r^x")]
    #[case("github:o/r%zz")]
    fn url_syntax_rejected(#[case] input: &str) {
        assert_matches!(
            FlakeRef::parse(input, Context::Standalone),
            Err(NixUriError::InvalidUrl(_))
        );
    }
}
