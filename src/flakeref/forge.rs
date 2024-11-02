use std::{fmt::Display, path::Path};

use nom::{
    branch::alt, bytes::complete::{tag, take_till, take_till1, take_until, take_while1}, combinator::{map, opt, rest, verify}, multi::many_m_n, sequence::tuple, IResult
};
use serde::{Deserialize, Serialize};

use crate::{
    error::{NixUriError, NixUriResult},
    parser::parse_url_type,
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
    // TODO?: Parse this incrementally. First get owner/repo, get Option</ref-rev>
    // TODO?: Apply gitlab/hub/sourcehut rule-checks
    /// Parses content of the form `/owner/repo/ref_or_rev`
    /// into an iterator akin to `vec![owner, repo, ref_or_rev].into_iter()`.
    pub(crate) fn parse_owner_repo_ref(input: &str) -> IResult<&str, impl Iterator<Item = &str>> {
        dbg!(input);
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
        let mut res = vec![owner, repo];
        // if the remaining is empty, that's the ref/rev
        if !maybe_refrev.is_empty() {
            res.push(maybe_refrev);
        }

        // TODO: return (&str, &str, Option<&str>) instead of an iterator
        Ok((tail, res.into_iter()))
    }
    pub fn parse(input: &str) -> IResult<&str, Self> {
        let (rest, platform) = GitForgePlatform::parse(input)?;
        let (rest, forge_path) = Self::parse_owner_repo_ref(rest)?;
        let mut forge_path = forge_path.map(String::from);
        let res = Self {
            platform,
            owner: forge_path.next().unwrap(),
            repo: forge_path.next().unwrap(),
            ref_or_rev: forge_path.next(),
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
                GitForgePlatform::GitHub => "github",
                GitForgePlatform::GitLab => "gitlab",
                GitForgePlatform::SourceHut => "sourcehut",
            }
        )
    }
}

#[cfg(test)]
mod inc_parse_platform {
    use super::*;
    use crate::parser::{parse_nix_uri, parse_params};

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
        // TODO?: fuzz test where `:` is preceeded by bad string
    }
}
#[cfg(test)]
mod inc_parse {
    use super::*;
    #[test]
    fn plain() {
        let input = "owner/repo";
        let (rest, mut iter) = GitForge::parse_owner_repo_ref(input).unwrap();
        assert_eq!(rest, "");
        assert_eq!(Some("owner"), iter.next());
        assert_eq!(Some("repo"), iter.next());
        assert_eq!(None, iter.next());
    }
    #[test]
    fn param_terminated() {
        let input = "owner/repo?🤡";
        let (rest, mut iter) = GitForge::parse_owner_repo_ref(input).unwrap();
        let parsed_out: Vec<_> = iter.collect();
        let expect_out = vec!["owner", "repo"];
        assert_eq!(parsed_out, expect_out);
        assert_eq!(rest, "?🤡");

        let input = "owner/repo#🤡";
        let (rest, mut iter) = GitForge::parse_owner_repo_ref(input).unwrap();
        let parsed_out: Vec<_> = iter.collect();
        assert_eq!(parsed_out, expect_out);
        assert_eq!(rest, "#🤡");

        let input = "owner/repo?#🤡";
        let (rest, mut iter) = GitForge::parse_owner_repo_ref(input).unwrap();
        let parsed_out: Vec<_> = iter.collect();
        assert_eq!(parsed_out, expect_out);
        assert_eq!(rest, "?#🤡");
    }

    #[test]
    fn attr_terminated() {
        let input = "owner/repo#fizz.bar";
        let (rest, mut iter) = GitForge::parse_owner_repo_ref(input).unwrap();
        assert_eq!(rest, "#fizz.bar");
        let parsed_out: Vec<_> = iter.collect();
        let expect_out = vec!["owner", "repo"];
        assert_eq!(parsed_out, expect_out);
    }

    #[test]
    fn rev_param_terminated() {
        let input = "owner/repo/rev?foo=bar";
        let (rest, mut iter) = GitForge::parse_owner_repo_ref(input).unwrap();
        assert_eq!(rest, "?foo=bar");
        let parsed_out: Vec<_> = iter.collect();
        let expect_out = vec!["owner", "repo", "rev"];
        assert_eq!(parsed_out, expect_out);
    }

    #[test]
    fn rev_attr_terminated() {
        let input = "owner/repo/rev#fizz.bar";
        let (rest, mut iter) = GitForge::parse_owner_repo_ref(input).unwrap();
        assert_eq!(rest, "#fizz.bar");
        let parsed_out: Vec<_> = iter.collect();
        let expect_out = vec!["owner", "repo", "rev"];
        assert_eq!(parsed_out, expect_out);
    }
}
