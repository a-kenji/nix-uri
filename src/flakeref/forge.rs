use std::fmt::Display;

use serde::{Deserialize, Serialize};
use winnow::{
    ModalResult, Parser,
    combinator::{alt, cut_err, opt, preceded, separated_pair, terminated},
    error::{ContextError, ErrMode, StrContext, StrContextValue},
    token::take_till,
};

use crate::{
    error::{NixUriError, NixUriResult, tag},
    flakeref::{
        RefLocation,
        validators::{looks_like_rev, validate_ref_name},
    },
};

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

impl GitForgePlatform {
    /// Parses the gitforge platform token: `<github|gitlab|sourcehut>`.
    #[allow(dead_code)]
    pub(crate) fn parse(input: &mut &str) -> ModalResult<Self> {
        alt((
            tag("github").value(Self::GitHub),
            tag("gitlab").value(Self::GitLab),
            tag("sourcehut").value(Self::SourceHut),
        ))
        .parse_next(input)
    }
    #[allow(dead_code)]
    pub(crate) fn parse_terminated(input: &mut &str) -> ModalResult<Self> {
        terminated(
            Self::parse,
            ':'.context(StrContext::Expected(StrContextValue::CharLiteral(':'))),
        )
        .parse_next(input)
    }
}

impl GitForge {
    /// `<owner>/<repo>[/?#]`
    fn parse_owner_repo<'i>(input: &mut &'i str) -> ModalResult<(&'i str, &'i str)> {
        cut_err(separated_pair(
            take_till(1.., |c: char| c == '/')
                .context(StrContext::Label("TakeTill1"))
                .context(StrContext::Label("owner")),
            '/'.context(StrContext::Expected(StrContextValue::CharLiteral('/'))),
            take_till(1.., |c: char| c == '/' || c == '?' || c == '#')
                .context(StrContext::Label("TakeTill1"))
                .context(StrContext::Label("repo")),
        ))
        .context(StrContext::Label("owner and repo"))
        .parse_next(input)
    }

    /// `/[foobar]<?#>...` -> `Option<foobar>`; consumes the leading `/` and
    /// optionally the trailing token before `?` / `#`.
    fn parse_rev_ref<'i>(input: &mut &'i str) -> ModalResult<Option<&'i str>> {
        preceded(
            '/'.context(StrContext::Expected(StrContextValue::CharLiteral('/'))),
            opt(take_till(1.., |c: char| c == '?' || c == '#')
                .context(StrContext::Label("TakeTill1"))),
        )
        .parse_next(input)
    }

    /// `<owner>/<repo>[/[value]] -> (owner, repo, value)`. The trailing
    /// `value`, when present, is classified by the caller into either
    /// `ref_` or `rev` via [`super::validators::looks_like_rev`].
    #[allow(dead_code)]
    pub(crate) fn parse_owner_repo_ref<'i>(
        input: &mut &'i str,
    ) -> ModalResult<(&'i str, &'i str, Option<&'i str>)> {
        let (owner, repo) = Self::parse_owner_repo(input)?;
        let maybe_refrev = opt(Self::parse_rev_ref).parse_next(input)?;
        Ok((owner, repo, maybe_refrev.flatten()))
    }

    #[allow(dead_code)]
    pub(crate) fn parse(input: &mut &str) -> ModalResult<Self> {
        let platform = terminated(
            GitForgePlatform::parse,
            ':'.context(StrContext::Expected(StrContextValue::CharLiteral(':'))),
        )
        .parse_next(input)?;
        let (owner, repo, maybe_value) = Self::parse_owner_repo_ref(input)?;
        let (ref_, rev) = match maybe_value {
            Some(v) if looks_like_rev(v) => (None, Some(v.to_string())),
            Some(v) if validate_ref_name(v) => (Some(v.to_string()), None),
            Some(_) => return Err(ErrMode::Cut(ContextError::new())),
            None => (None, None),
        };
        Ok(Self {
            platform,
            owner: owner.to_string(),
            repo: repo.to_string(),
            ref_,
            rev,
            location: RefLocation::PathComponent,
        })
    }
}

/// `[a-zA-Z0-9._-]` is the strictest common alphabet across `github:`,
/// `gitlab:`, and `sourcehut:`. `SourceHut` additionally permits a leading
/// `~` on owner (e.g. `~misterio/nix-colors`); that is the only platform-
/// specific carve-out.
fn is_owner_repo_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

/// Reject owner/repo strings that upstream Nix would accept syntactically
/// but that no real forge would resolve. Lets parse-time errors stand in
/// for the fetch-time failure a downstream consumer would otherwise see.
///
/// `owner` and `repo` are the post-percent-decode values the parser pulled
/// out of the URL path; see [`super::fr_type::FlakeRefType::parse_type`]'s
/// `GitForge` arm. A `gitlab:` owner is allowed to carry decoded `/` so
/// nested-subgroup forms like `gitlab:veloren/dev/rfcs` (encoded
/// `gitlab:veloren%2Fdev/rfcs` on the wire) round-trip; GitHub and
/// `SourceHut` do not have a subgroup concept upstream, so the same input
/// still rejects there.
///
/// The `field` name is `"owner"` or `"repo"` so consumers can pattern-match
/// on which segment failed.
pub(crate) fn validate_owner_repo(
    platform: &GitForgePlatform,
    owner: &str,
    repo: &str,
) -> NixUriResult<()> {
    let owner_body = if matches!(platform, GitForgePlatform::SourceHut) {
        owner.strip_prefix('~').unwrap_or(owner)
    } else {
        owner
    };
    if owner_body.is_empty() {
        return Err(NixUriError::InvalidValue {
            field: "owner",
            reason: "owner must not be empty".to_string(),
        });
    }
    if owner_body.contains('/') {
        if !matches!(platform, GitForgePlatform::GitLab) {
            return Err(NixUriError::InvalidValue {
                field: "owner",
                reason: "only gitlab owners may contain a '/' (subgroup form)".to_string(),
            });
        }
        for segment in owner_body.split('/') {
            if segment.is_empty() {
                return Err(NixUriError::InvalidValue {
                    field: "owner",
                    reason: "subgroup owner must not have empty segments or leading/trailing '/'"
                        .to_string(),
                });
            }
            let first = segment.chars().next().unwrap();
            if first == '-' || first == '.' {
                return Err(NixUriError::InvalidValue {
                    field: "owner",
                    reason: "owner segment must not start with '-' or '.'".to_string(),
                });
            }
            if !segment.chars().all(is_owner_repo_char) {
                return Err(NixUriError::InvalidValue {
                    field: "owner",
                    reason: "owner segment contains a character outside [a-zA-Z0-9._-]".to_string(),
                });
            }
        }
    } else {
        let owner_first = owner_body.chars().next().unwrap();
        if owner_first == '-' || owner_first == '.' {
            return Err(NixUriError::InvalidValue {
                field: "owner",
                reason: "owner must not start with '-' or '.'".to_string(),
            });
        }
        if !owner_body.chars().all(is_owner_repo_char) {
            return Err(NixUriError::InvalidValue {
                field: "owner",
                reason: "owner contains a character outside [a-zA-Z0-9._-]".to_string(),
            });
        }
    }
    if repo.is_empty() {
        return Err(NixUriError::InvalidValue {
            field: "repo",
            reason: "repo must not be empty".to_string(),
        });
    }
    if repo.starts_with('.') {
        return Err(NixUriError::InvalidValue {
            field: "repo",
            reason: "repo must not start with '.'".to_string(),
        });
    }
    if !repo.chars().all(is_owner_repo_char) {
        return Err(NixUriError::InvalidValue {
            field: "repo",
            reason: "repo contains a character outside [a-zA-Z0-9._-]".to_string(),
        });
    }
    Ok(())
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
mod inc_parse_platform {
    use super::*;

    #[test]
    fn platform() {
        let remain = ":nixos/nixpkgs";

        let uri = "github:nixos/nixpkgs";

        let (rest, platform) = GitForgePlatform::parse.parse_peek(uri).unwrap();
        assert_eq!(rest, remain);
        assert_eq!(platform, GitForgePlatform::GitHub);

        let (rest, platform) = GitForgePlatform::parse_terminated.parse_peek(uri).unwrap();
        assert_eq!(rest, &remain[1..]);
        assert_eq!(platform, GitForgePlatform::GitHub);

        let uri = "gitlab:nixos/nixpkgs";

        let (rest, platform) = GitForgePlatform::parse.parse_peek(uri).unwrap();
        assert_eq!(rest, remain);
        assert_eq!(platform, GitForgePlatform::GitLab);

        let uri = "sourcehut:nixos/nixpkgs";

        let (rest, platform) = GitForgePlatform::parse.parse_peek(uri).unwrap();
        assert_eq!(rest, remain);
        assert_eq!(platform, GitForgePlatform::SourceHut);
        // TODO?: fuzz test where `:` is preceded by bad string
    }
}

#[cfg(test)]
mod err_msgs {
    use cool_asserts::assert_matches;

    #[test]
    fn just_owner_public_surface() {
        use crate::{NixUriError, ParseExpected, parser::parse_nix_uri};

        assert_matches!(
            parse_nix_uri("github:owner"),
            Err(NixUriError::Parse {
                position: 12,
                expected: ParseExpected::Char('/'),
            })
        );
    }

    #[test]
    fn whitespace_in_owner_rejected() {
        use crate::{NixUriError, parser::parse_nix_uri};
        assert_matches!(
            parse_nix_uri("github:bad owner/repo"),
            Err(NixUriError::InvalidValue { field: "owner", .. })
        );
    }

    #[test]
    fn whitespace_in_repo_rejected() {
        use crate::{NixUriError, parser::parse_nix_uri};
        assert_matches!(
            parse_nix_uri("github:owner/bad repo"),
            Err(NixUriError::InvalidValue { field: "repo", .. })
        );
    }

    #[test]
    fn leading_dot_owner_rejected() {
        use crate::{NixUriError, parser::parse_nix_uri};
        assert_matches!(
            parse_nix_uri("github:.dotted/repo"),
            Err(NixUriError::InvalidValue { field: "owner", .. })
        );
    }

    #[test]
    fn leading_dash_owner_rejected() {
        use crate::{NixUriError, parser::parse_nix_uri};
        assert_matches!(
            parse_nix_uri("github:-dashed/repo"),
            Err(NixUriError::InvalidValue { field: "owner", .. })
        );
    }

    #[test]
    fn leading_dot_repo_rejected() {
        use crate::{NixUriError, parser::parse_nix_uri};
        assert_matches!(
            parse_nix_uri("github:owner/.dotrepo"),
            Err(NixUriError::InvalidValue { field: "repo", .. })
        );
    }

    #[test]
    fn special_char_in_owner_rejected() {
        use crate::{NixUriError, parser::parse_nix_uri};
        assert_matches!(
            parse_nix_uri("github:bad!owner/repo"),
            Err(NixUriError::InvalidValue { field: "owner", .. })
        );
    }

    #[test]
    fn tilde_in_github_owner_rejected() {
        use crate::{NixUriError, parser::parse_nix_uri};
        assert_matches!(
            parse_nix_uri("github:~tilde/repo"),
            Err(NixUriError::InvalidValue { field: "owner", .. })
        );
    }

    #[test]
    fn tilde_only_in_sourcehut_owner() {
        use crate::parser::parse_nix_uri;
        // SourceHut owner permits a leading `~` (e.g. `~misterio`).
        parse_nix_uri("sourcehut:~owner/repo").expect("sourcehut owner with `~` is valid");
        // GitLab and GitHub do not.
        assert!(parse_nix_uri("gitlab:~owner/repo").is_err());
        assert!(parse_nix_uri("github:~owner/repo").is_err());
    }

    #[test]
    fn valid_forms_still_accepted() {
        use crate::parser::parse_nix_uri;
        for uri in [
            "github:nixos/nixpkgs",
            "github:nix.os/nix-pkgs",
            "github:n_ix/r_epo",
            "github:o-1/r.2",
            "gitlab:owner/repo",
            "sourcehut:nixos/nixpkgs",
            "sourcehut:~misterio/nix-colors",
        ] {
            parse_nix_uri(uri).unwrap_or_else(|e| panic!("expected {uri:?} to parse, got {e}"));
        }
    }

    /// GitLab nested subgroups are written as
    /// `gitlab:veloren%2Fdev/rfcs` in Nix. The parser percent-decodes
    /// each path segment after splitting on the literal `/`, so `%2F`
    /// survives as a literal `/` inside the owner without colliding
    /// with the owner-vs-repo boundary.
    #[test]
    fn gitlab_subgroup_percent_decode() {
        use crate::{FlakeRef, FlakeRefType, GitForge, GitForgePlatform, parser::parse_nix_uri};
        let uri = "gitlab:veloren%2Fdev/rfcs";
        let parsed: FlakeRef = parse_nix_uri(uri).expect("subgroup form should parse");
        match parsed.kind() {
            FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitLab,
                owner,
                repo,
                ..
            }) => {
                assert_eq!(owner, "veloren/dev");
                assert_eq!(repo, "rfcs");
            }
            other => panic!("expected GitLab GitForge, got {other:?}"),
        }
        // Display re-encodes the `/` in owner so the wire form matches the input
        // and the round-trip stays byte-stable.
        assert_eq!(parsed.to_string(), uri);
    }

    #[test]
    fn gitlab_deep_subgroup() {
        use crate::{FlakeRefType, parser::parse_nix_uri};
        let uri = "gitlab:o%2Fp%2Fq/r";
        let parsed = parse_nix_uri(uri).expect("deep subgroup should parse");
        match parsed.kind() {
            FlakeRefType::GitForge(g) => {
                assert_eq!(g.owner, "o/p/q");
                assert_eq!(g.repo, "r");
            }
            other => panic!("expected GitForge, got {other:?}"),
        }
        assert_eq!(parsed.to_string(), uri);
    }

    #[test]
    fn gitlab_subgroup_with_ref_query() {
        use crate::{FlakeRefType, parser::parse_nix_uri};
        let parsed =
            parse_nix_uri("gitlab:o%2Fp/r?ref=main").expect("ref-query subgroup should parse");
        match parsed.kind() {
            FlakeRefType::GitForge(g) => {
                assert_eq!(g.owner, "o/p");
                assert_eq!(g.repo, "r");
                assert_eq!(g.ref_.as_deref(), Some("main"));
            }
            other => panic!("expected GitForge, got {other:?}"),
        }
    }

    #[test]
    fn gitlab_subgroup_with_path_rev() {
        use crate::{FlakeRefType, parser::parse_nix_uri};
        let rev = "0123456789abcdef0123456789abcdef01234567";
        let parsed = parse_nix_uri(&format!("gitlab:o%2Fp/r/{rev}"))
            .expect("path-rev subgroup should parse");
        match parsed.kind() {
            FlakeRefType::GitForge(g) => {
                assert_eq!(g.owner, "o/p");
                assert_eq!(g.rev.as_deref(), Some(rev));
                assert_eq!(g.ref_, None);
            }
            other => panic!("expected GitForge, got {other:?}"),
        }
    }

    #[test]
    fn gitlab_leading_slash_in_owner_rejected() {
        use crate::{NixUriError, parser::parse_nix_uri};
        assert_matches!(
            parse_nix_uri("gitlab:%2Fp/r"),
            Err(NixUriError::InvalidValue { field: "owner", .. })
        );
    }

    #[test]
    fn gitlab_trailing_slash_in_owner_rejected() {
        use crate::{NixUriError, parser::parse_nix_uri};
        assert_matches!(
            parse_nix_uri("gitlab:p%2F/r"),
            Err(NixUriError::InvalidValue { field: "owner", .. })
        );
    }

    #[test]
    fn gitlab_empty_subgroup_segment_rejected() {
        use crate::{NixUriError, parser::parse_nix_uri};
        assert_matches!(
            parse_nix_uri("gitlab:p%2F%2Fq/r"),
            Err(NixUriError::InvalidValue { field: "owner", .. })
        );
    }

    /// GitHub has no subgroup concept and `validate_owner_repo` is the
    /// parse-time stand-in for the fetch-time failure a downstream
    /// consumer would otherwise see, so reject the subgroup form here
    /// even though Nix accepts it syntactically (Nix applies the same
    /// path-segment decode regardless of forge); the platform gate is a
    /// nix-uri-only safety.
    #[test]
    fn github_subgroup_owner_rejected() {
        use crate::{NixUriError, parser::parse_nix_uri};
        assert_matches!(
            parse_nix_uri("github:o%2Fp/r"),
            Err(NixUriError::InvalidValue { field: "owner", .. })
        );
    }

    #[test]
    fn sourcehut_subgroup_owner_rejected() {
        use crate::{NixUriError, parser::parse_nix_uri};
        assert_matches!(
            parse_nix_uri("sourcehut:o%2Fp/r"),
            Err(NixUriError::InvalidValue { field: "owner", .. })
        );
    }
}

#[cfg(test)]
mod inc_parse {
    use super::*;

    #[test]
    fn plain() {
        let input = "owner/repo";
        let (rest, res) = GitForge::parse_owner_repo_ref.parse_peek(input).unwrap();
        let expected = ("owner", "repo", None);
        assert_eq!(rest, "");
        assert_eq!(expected, res);
    }

    #[test]
    fn param_terminated() {
        let input = "owner/repo?🤡";
        let (rest, res) = GitForge::parse_owner_repo_ref.parse_peek(input).unwrap();
        let expected = ("owner", "repo", None);
        assert_eq!(rest, "?🤡");
        assert_eq!(expected, res);
        assert_eq!(rest, "?🤡");

        let input = "owner/repo#🤡";
        let (rest, res) = GitForge::parse_owner_repo_ref.parse_peek(input).unwrap();
        let expected = ("owner", "repo", None);
        assert_eq!(expected, res);
        assert_eq!(rest, "#🤡");

        let input = "owner/repo?#🤡";
        let (rest, res) = GitForge::parse_owner_repo_ref.parse_peek(input).unwrap();
        let expected = ("owner", "repo", None);
        assert_eq!(expected, res);
        assert_eq!(rest, "?#🤡");
    }

    #[test]
    fn attr_terminated() {
        let input = "owner/repo#fizz.bar";
        let (rest, res) = GitForge::parse_owner_repo_ref.parse_peek(input).unwrap();
        let expected = ("owner", "repo", None);
        assert_eq!(rest, "#fizz.bar");
        assert_eq!(expected, res);
    }

    #[test]
    fn rev_param_terminated() {
        let input = "owner/repo/rev?foo=bar";
        let (rest, res) = GitForge::parse_owner_repo_ref.parse_peek(input).unwrap();
        let expected = ("owner", "repo", Some("rev"));
        assert_eq!(rest, "?foo=bar");
        assert_eq!(expected, res);
    }

    #[test]
    fn rev_attr_terminated() {
        let input = "owner/repo/rev#fizz.bar";
        let (rest, res) = GitForge::parse_owner_repo_ref.parse_peek(input).unwrap();
        let expected = ("owner", "repo", Some("rev"));
        assert_eq!(rest, "#fizz.bar");
        assert_eq!(expected, res);
    }
}
