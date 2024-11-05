use std::fmt::Display;

use nom::{
    branch::alt,
    bytes::complete::{tag, take_till, take_till1},
    combinator::{map, opt},
    IResult,
};
use serde::{Deserialize, Serialize};


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
    fn parse_hub(input: &str) -> IResult<&str, Self> {
        map(tag("github"), |_| Self::GitHub)(input)
    }
    fn parse_lab(input: &str) -> IResult<&str, Self> {
        map(tag("gitlab"), |_| Self::GitLab)(input)
    }
    fn parse_sourcehut(input: &str) -> IResult<&str, Self> {
        map(tag("sourcehut"), |_| Self::SourceHut)(input)
    }
    /// `nom`s the gitforge + `:`
    /// `"<github|gitlab|sourceforge>:foobar..."` -> `(foobar..., GitForge)`
    pub fn parse(input: &str) -> IResult<&str, Self> {
        let (rest, res) = alt((Self::parse_hub, Self::parse_lab, Self::parse_sourcehut))(input)?;
        let (rest, _) = tag(":")(rest)?;
        Ok((rest, res))
    }
}

impl GitForge {
    // TODO?: Apply gitlab/hub/sourcehut rule-checks
    // TODO: #158
    // TODO: #163
    /// <owner>/<repo>[/[ref-or-rev]] -> (owner: &str, repo: &str, ref_or_rev: Option<&str>)
    pub(crate) fn parse_owner_repo_ref(input: &str) -> IResult<&str, (&str, &str, Option<&str>)> {
        // pull out the component we are parsing
        let (tail, path0) = take_till(|c| c == '#' || c == '?')(input)?;
        // pull out the owner
        let (path1, owner) = take_till1(|c| c == '/')(path0)?;
        // ...and discard the `/` separator
        let (path1, _) = tag("/")(path1)?;
        // get the rest, halting at the optional `/`
        let (path2, repo) = take_till1(|c| c == '/')(path1)?;
        // drop the `/` if it exists
        let (maybe_refrev, _) = opt(tag("/"))(path2)?;
        // if the remaining is empty, that's the ref/rev
        let maybe_refrev = if maybe_refrev.is_empty() {
            None
        } else {
            Some(maybe_refrev)
        };

        Ok((tail, (owner, repo, maybe_refrev)))
    }
    pub fn parse(input: &str) -> IResult<&str, Self> {
        let (rest, platform) = GitForgePlatform::parse(input)?;
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
        let stripped = "nixos/nixpkgs";

        let uri = "github:nixos/nixpkgs";
        let (rest, platform) = GitForgePlatform::parse(uri).unwrap();
        assert_eq!(rest, stripped);
        assert_eq!(platform, GitForgePlatform::GitHub);

        let uri = "gitlab:nixos/nixpkgs";
        let (rest, platform) = GitForgePlatform::parse(uri).unwrap();
        assert_eq!(rest, stripped);
        assert_eq!(platform, GitForgePlatform::GitLab);

        let uri = "sourcehut:nixos/nixpkgs";
        let (rest, platform) = GitForgePlatform::parse(uri).unwrap();
        assert_eq!(rest, stripped);
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
        let input = "owner";
        let input_slash = "owner/";

        let _err = GitForge::parse_owner_repo_ref(input).unwrap_err();
        let _err_slash = GitForge::parse_owner_repo_ref(input_slash).unwrap_err();

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
