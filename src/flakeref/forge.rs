use std::fmt::Display;

use serde::{Deserialize, Serialize};
use winnow::{
    combinator::{alt, cut_err, opt, terminated, trace},
    error::{StrContext, StrContextValue},
    token::take_till,
    PResult, Parser,
};

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
    pub fn parse(input: &mut &str) -> PResult<Self> {
        let res = alt((
            trace("gitforge: github", "github".value(Self::GitHub)),
            trace("gitforge: gitlab", "gitlab".value(Self::GitLab)),
            trace("gitforge: sourcehut", "sourcehut".value(Self::SourceHut)),
        ))
        .context(StrContext::Expected(StrContextValue::Description(
            "gitforge|github|gitlab",
        )))
        .parse_next(input)?;
        Ok(res)
    }
    pub fn parse_terminated(input: &mut &str) -> PResult<Self> {
        terminated(Self::parse, ":")
            .context(StrContext::Expected(StrContextValue::CharLiteral(':')))
            .parse_next(input)
    }
}

impl GitForge {
    pub(crate) fn parse_owner_repo<'i>(input: &mut &'i str) -> PResult<(&'i str, &'i str)> {
        let owner = trace(
            "til '/' for owner",
            cut_err(take_till(1.., |c| c == '/').context(StrContext::Expected(
                StrContextValue::StringLiteral("<owner>"),
            ))),
        )
        .parse_next(input)?;
        let _ = cut_err("/")
            .context(StrContext::Expected(StrContextValue::CharLiteral('/')))
            .parse_next(input)?;
        // get the rest, halting at the optional `/`
        let repo = trace(
            "till '/#?' for repo",
            cut_err(take_till(1.., |c| c == '/' || c == '#' || c == '?')),
        )
        .context(StrContext::Expected(StrContextValue::StringLiteral(
            "<repo>",
        )))
        .parse_next(input)?;
        let _ = opt("/").parse_next(input)?;
        Ok((owner, repo))
    }
    // TODO?: Apply gitlab/hub/sourcehut rule-checks
    // TODO: #158
    // TODO: #163
    /// <owner>/<repo>[/[ref-or-rev]] -> (owner: &str, repo: &str, ref_or_rev: Option<&str>)
    pub(crate) fn parse_owner_repo_ref<'i>(
        input: &mut &'i str,
    ) -> PResult<(&'i str, &'i str, Option<&'i str>)> {
        // pull out the owner
        let (owner, repo) = Self::parse_owner_repo
            .context(StrContext::Label("owner/repo"))
            .parse_next(input)?;

        // drop the `/` if it exists
        let maybe_refrev =
            trace("till '#?'", opt(take_till(1.., |c| c == '#' || c == '?'))).parse_next(input)?;
        // if the remaining is empty, that's the ref/rev

        Ok((owner, repo, maybe_refrev))
    }

    pub fn parse(input: &mut &str) -> PResult<Self> {
        let platform = GitForgePlatform::parse_terminated(input)?;
        // If we parsed the platform, but fail on owner/repo, don't try a backtrack
        let forge_path = cut_err(Self::parse_owner_repo_ref).parse_next(input)?;
        let res = Self {
            platform,
            owner: forge_path.0.to_string(),
            repo: forge_path.1.to_string(),
            ref_or_rev: forge_path.2.map(str::to_string),
        };
        Ok(res)
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
        let stripped = "nixos/nixpkgs";

        let mut uri = "github:nixos/nixpkgs";

        let platform = GitForgePlatform::parse_terminated(&mut uri).unwrap();
        assert_eq!(uri, stripped);
        assert_eq!(platform, GitForgePlatform::GitHub);

        let mut uri = "gitlab:nixos/nixpkgs";

        let platform = GitForgePlatform::parse_terminated(&mut uri).unwrap();
        assert_eq!(uri, stripped);
        assert_eq!(platform, GitForgePlatform::GitLab);

        let mut uri = "sourcehut:nixos/nixpkgs";

        let platform = GitForgePlatform::parse_terminated(&mut uri).unwrap();
        assert_eq!(uri, stripped);
        assert_eq!(platform, GitForgePlatform::SourceHut);
        // TODO?: fuzz test where `:` is preceded by bad string
    }
}
#[cfg(test)]
mod err_msgs {
    use super::*;
    #[test]
    #[ignore = "partial owner-repo parsing not yet implemented"]
    fn just_owner() {
        let mut input = "owner";
        let mut input_slash = "owner/";

        let _err = GitForge::parse_owner_repo_ref(&mut input).unwrap_err();
        let _err_slash = GitForge::parse_owner_repo_ref(&mut input_slash).unwrap_err();

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
        let mut input = "owner/repo";
        let res = GitForge::parse_owner_repo_ref(&mut input).unwrap();
        let expected = ("owner", "repo", None);
        assert_eq!("", input);
        assert_eq!(expected, res);
    }

    #[test]
    fn param_terminated() {
        let mut input = "owner/repo?🤡";
        let res = GitForge::parse_owner_repo_ref(&mut input).unwrap();
        let expected = ("owner", "repo", None);
        assert_eq!("?🤡", input);
        assert_eq!(expected, res);

        let mut input = "owner/repo#🤡";
        let res = GitForge::parse_owner_repo_ref(&mut input).unwrap();
        let expected = ("owner", "repo", None);

        assert_eq!("#🤡", input);
        assert_eq!(expected, res);

        let mut input = "owner/repo?#🤡";
        let res = GitForge::parse_owner_repo_ref(&mut input).unwrap();
        let expected = ("owner", "repo", None);
        assert_eq!("?#🤡", input);
        assert_eq!(expected, res);
    }

    #[test]
    fn attr_terminated() {
        let mut input = "owner/repo#fizz.bar";
        let res = GitForge::parse_owner_repo_ref(&mut input).unwrap();
        let expected = ("owner", "repo", None);
        assert_eq!("#fizz.bar", input);
        assert_eq!(expected, res);
    }

    #[test]
    fn rev_param_terminated() {
        let mut input = "owner/repo/rev?foo=bar";
        let res = GitForge::parse_owner_repo_ref(&mut input).unwrap();
        let expected = ("owner", "repo", Some("rev"));
        assert_eq!("?foo=bar", input);
        assert_eq!(expected, res);
    }

    #[test]
    fn rev_attr_terminated() {
        let mut input = "owner/repo/rev#fizz.bar";
        let res = GitForge::parse_owner_repo_ref(&mut input).unwrap();
        let expected = ("owner", "repo", Some("rev"));
        assert_eq!("#fizz.bar", input);
        assert_eq!(expected, res);
    }
}
