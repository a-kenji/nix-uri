use std::fmt::Display;

use nom::{
    branch::alt,
    bytes::complete::take_till1,
    character::complete::char,
    combinator::{cut, opt, value},
    error::context,
    sequence::{preceded, separated_pair, terminated},
    IResult,
};
use nom_supreme::tag::complete::tag;
use serde::{Deserialize, Serialize};

use crate::IErr;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum GitForgePlatform {
    GitHub,
    GitLab,
    SourceHut,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitForge {
    pub platform: GitForgePlatform,
    pub owner: String,
    pub repo: String,
    pub ref_or_rev: Option<String>,
}

impl GitForgePlatform {
    /// `nom`s the gitforge + `:`
    /// `"<github|gitlab|sourceforge>:foobar..."` -> `(foobar..., GitForge)`
    pub fn parse(input: &str) -> IResult<&str, Self, IErr<&str>> {
        alt((
            value(Self::GitHub, tag("github")),
            value(Self::GitLab, tag("gitlab")),
            value(Self::SourceHut, tag("sourcehut")),
        ))(input)
    }
    pub fn parse_terminated(input: &str) -> IResult<&str, Self, IErr<&str>> {
        terminated(Self::parse, char(':'))(input)
    }
}

impl GitForge {
    /// <owner>/<repo>[/?#]
    // TODO: return up a `NixUIError::MissingTypeParameter
    fn parse_owner_repo(input: &str) -> IResult<&str, (&str, &str), IErr<&str>> {
        context(
            "owner and repo",
            cut(separated_pair(
                context("owner", take_till1(|c| c == '/')),
                char('/'),
                context("repo", take_till1(|c| c == '/' || c == '?' || c == '#')),
            )),
        )(input)
    }

    /// `/[foobar]<?#>...` -> `(<?#>...), Option<foobar>)`
    fn parse_rev_ref(input: &str) -> IResult<&str, Option<&str>, IErr<&str>> {
        preceded(char('/'), opt(take_till1(|c| c == '?' || c == '#')))(input)
    }
    // TODO?: Apply gitlab/hub/sourcehut rule-checks
    // TODO: #158
    // TODO: #163
    /// <owner>/<repo>[/[ref-or-rev]] -> (owner: &str, repo: &str, ref_or_rev: Option<&str>)
    pub(crate) fn parse_owner_repo_ref(
        input: &str,
    ) -> IResult<&str, (&str, &str, Option<&str>), IErr<&str>> {
        let (input, (owner, repo)) = Self::parse_owner_repo(input)?;
        // drop the `/` if it exists
        let (input, maybe_refrev) = opt(Self::parse_rev_ref)(input)?;
        // if the remaining is empty, that's the ref/rev

        Ok((input, (owner, repo, maybe_refrev.flatten())))
    }
    pub fn parse(input: &str) -> IResult<&str, Self, IErr<&str>> {
        let (rest, platform) = terminated(GitForgePlatform::parse, char(':'))(input)?;
        let (rest, forge_path) = Self::parse_owner_repo_ref(rest)?;
        let res = Self {
            platform,
            owner: forge_path.0.to_string(),
            repo: forge_path.1.to_string(),
            ref_or_rev: forge_path.2.map(str::to_string),
        };
        Ok((rest, res))
    }
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

        let (rest, platform) = GitForgePlatform::parse(uri).unwrap();
        assert_eq!(rest, remain);
        assert_eq!(platform, GitForgePlatform::GitHub);

        let (rest, platform) = GitForgePlatform::parse_terminated(uri).unwrap();
        assert_eq!(rest, &remain[1..]);
        assert_eq!(platform, GitForgePlatform::GitHub);

        let uri = "gitlab:nixos/nixpkgs";

        let (rest, platform) = GitForgePlatform::parse(uri).unwrap();
        assert_eq!(rest, remain);
        assert_eq!(platform, GitForgePlatform::GitLab);

        let uri = "sourcehut:nixos/nixpkgs";

        let (rest, platform) = GitForgePlatform::parse(uri).unwrap();
        assert_eq!(rest, remain);
        assert_eq!(platform, GitForgePlatform::SourceHut);
        // TODO?: fuzz test where `:` is preceded by bad string
    }
}
#[cfg(test)]
mod err_msgs {
    use cool_asserts::assert_matches;
    use nom::{error::ErrorKind, Finish};
    use nom_supreme::error::{BaseErrorKind, ErrorTree, Expectation, StackContext};

    use super::*;
    #[test]
    fn just_owner() {
        let input = "owner";
        let input_slash = "owner/";

        let err = GitForge::parse_owner_repo_ref(input).finish().unwrap_err();
        // panic!("{:?}", err);
        assert_matches!(
            err,
            ErrorTree::Stack {
                base, //: Box(ErrorTree::Base {location, kind}),
                contexts,
            } => {
                assert_matches!(*base, ErrorTree::Base {
                    location: "",
                    kind: BaseErrorKind::Expected(Expectation::Char('/'))
                });
                assert_eq!(contexts, [
                    ("owner", StackContext::Context("owner and repo")),
                ]);
            }
        );
        let err_slash = GitForge::parse_owner_repo_ref(input_slash)
            .finish()
            .unwrap_err();
        assert_matches!(
            err_slash,
            ErrorTree::Stack {
                base, //: Box(ErrorTree::Base {location, kind}),
                contexts,
            } => {
                assert_matches!(*base, ErrorTree::Base {
                    location: "",
                    kind: BaseErrorKind::Kind(ErrorKind::TakeTill1)
                });
                assert_eq!(contexts, [
                    ("", StackContext::Context("repo")),
                    ("owner/", StackContext::Context("owner and repo")),
                ]);
            }
        );

        // assert_eq!(input, expected  `/` is missing);
        // assert_eq!(input, expected repo-string is missing);
    }
    #[test]
    #[ignore = "bad github ownerstring not yet impld"]
    fn git_owner() {
        let _input = "bad-owner/";

        // let err = GitForge::parse_owner_repo_ref(input, GitForgePlatform::GitHub).unwrap_err();
        // assert_eq!(input, invalid github owner format);
    }
    #[test]
    #[ignore = "bad github repostring not yet impld"]
    fn git_repo() {
        let _input = "owner/bad-string";

        // let err = GitForge::parse_owner_repo_ref(input, GitForgePlatform::GitHub).unwrap_err();
        // assert_eq!(input, invalid github owner format);
    }
    #[test]
    #[ignore = "bad mercurial ownerstring not yet impld"]
    fn merc_owner() {
        let _input = "bad-owner/";

        // let err = GitForge::parse_owner_repo_ref(input, GitForgePlatform::Mercurial).unwrap_err();
        // assert_eq!(input, invalid github owner format);
    }
    #[test]
    #[ignore = "bad mercurial repostring not yet impld"]
    fn merc_repo() {
        let _input = "owner/bad-string";

        // let err = GitForge::parse_owner_repo_ref(input, GitForgePlatform::Mercurial).unwrap_err();
        // assert_eq!(input, invalid github owner format);
    }
}
#[cfg(test)]
mod inc_parse {
    use super::*;

    #[test]
    fn plain() {
        let input = "owner/repo";
        let (rest, res) = GitForge::parse_owner_repo_ref(input).unwrap();
        let expected = ("owner", "repo", None);
        assert_eq!(rest, "");
        assert_eq!(expected, res);
    }

    #[test]
    fn param_terminated() {
        let input = "owner/repo?🤡";
        let (rest, res) = GitForge::parse_owner_repo_ref(input).unwrap();
        let expected = ("owner", "repo", None);
        assert_eq!(rest, "?🤡");
        assert_eq!(expected, res);
        assert_eq!(rest, "?🤡");

        let input = "owner/repo#🤡";
        let (rest, res) = GitForge::parse_owner_repo_ref(input).unwrap();
        let expected = ("owner", "repo", None);
        assert_eq!(expected, res);
        assert_eq!(rest, "#🤡");

        let input = "owner/repo?#🤡";
        let (rest, res) = GitForge::parse_owner_repo_ref(input).unwrap();
        let expected = ("owner", "repo", None);
        assert_eq!(expected, res);
        assert_eq!(rest, "?#🤡");
    }

    #[test]
    fn attr_terminated() {
        let input = "owner/repo#fizz.bar";
        let (rest, res) = GitForge::parse_owner_repo_ref(input).unwrap();
        let expected = ("owner", "repo", None);
        assert_eq!(rest, "#fizz.bar");
        assert_eq!(expected, res);
    }

    #[test]
    fn rev_param_terminated() {
        let input = "owner/repo/rev?foo=bar";
        let (rest, res) = GitForge::parse_owner_repo_ref(input).unwrap();
        let expected = ("owner", "repo", Some("rev"));
        assert_eq!(rest, "?foo=bar");
        assert_eq!(expected, res);
    }

    #[test]
    fn rev_attr_terminated() {
        let input = "owner/repo/rev#fizz.bar";
        let (rest, res) = GitForge::parse_owner_repo_ref(input).unwrap();
        let expected = ("owner", "repo", Some("rev"));
        assert_eq!(rest, "#fizz.bar");
        assert_eq!(expected, res);
    }
}
