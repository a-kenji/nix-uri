use std::fmt::Display;

use serde::{Deserialize, Serialize};

use crate::flakeref::RefLocation;

/// Which git-forge scheme a `GitForge` reference uses. Spelled in the URL as
/// the leading `github:`, `gitlab:`, or `sourcehut:` token; also drives the
/// canonical-domain lookup in [`super::ForgeIdentity`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum GitForgePlatform {
    GitHub,
    GitLab,
    SourceHut,
}

/// A reference into a git forge (`github:`, `gitlab:`, `sourcehut:`).
///
/// Ref and rev are stored as separate typed slots; the parser splits a
/// path-component value (`github:owner/repo/<x>`) into `rev` if `<x>` is
/// 40-hex, otherwise into `ref_`. The
/// `location` field records where the value would be rendered on `Display`,
/// so round-trips preserve `?ref=` vs `/ref` form. The "no ref and no rev"
/// state is encoded by both fields being `None`; `location` still defaults
/// to `PathComponent` for that case.
///
/// `#[non_exhaustive]` reserves room for future fields (e.g. `host`,
/// `submodules`) to land here without breaking match arms downstream.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct GitForge {
    pub platform: GitForgePlatform,
    pub owner: String,
    pub repo: String,
    pub ref_: Option<String>,
    pub rev: Option<String>,
    pub location: RefLocation,
}

impl Display for GitForgePlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::GitHub => "github",
                Self::GitLab => "gitlab",
                Self::SourceHut => "sourcehut",
            }
        )
    }
}

#[cfg(test)]
mod owner_repo {
    //! Git-forge owners and repos are not validated beyond URL syntax, and
    //! each segment is percent-decoded after splitting on `/`.
    use cool_asserts::assert_matches;
    use rstest::rstest;

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
    #[case("github:bad!owner/repo", "bad!owner", "repo", "github:bad!owner/repo")]
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
            Err(NixUriError::Unsupported(UnsupportedReason::MissingParameter { parameter, .. })) => {
                assert_eq!(parameter, "repo");
            }
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
