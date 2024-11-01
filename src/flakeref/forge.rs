use std::{fmt::Display, path::Path};

use nom::{
    branch::alt,
    bytes::complete::{tag, take_until},
    combinator::{map, opt, rest},
    multi::many_m_n,
    IResult,
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
    platform: GitForgePlatform,
    owner: String,
    repo: String,
    ref_or_rev: Option<String>,
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
    /// Parses content of the form `/owner/repo/ref_or_rev`
    /// into an iterator akin to `vec![owner, repo, ref_or_rev].into_iter()`.
    pub(crate) fn parse_owner_repo_ref(input: &str) -> IResult<&str, impl Iterator<Item = &str>> {
        use nom::sequence::separated_pair;
        let (input, owner_or_ref) = many_m_n(
            0,
            3,
            separated_pair(
                take_until("/"),
                tag("/"),
                alt((take_until("/"), take_until("?"), take_until("#"), rest)),
            ),
        )(input)?;

        let owner_and_rev_or_ref = owner_or_ref
            .clone()
            .into_iter()
            .flat_map(|(x, y)| vec![x, y])
            .filter(|s| !s.is_empty());
        Ok((input, owner_and_rev_or_ref))
    }

    // pub fn parse(input: &str) -> IResult<&str, Self> {
    //     // <platform>:... -> ...
    //     let (rest, platform) = GitForgePlatform::parse(input)?;
    //     // <owner>/<repo>... -> <repo>...
    //     let (rest, owner) = take_until(tag("/"))(rest)?;
    //     let (rest, _) = tag("/")(rest)?;
    //
    //     // <repo>[/rev-refg | ?opts | #attrs] -> [/rev-refg | ?opts | #attrs]
    //     let (rest, repo) = take_until(alt((tag("/"), tag("?"), tag("#"))))(rest)?;
    // }
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
mod incremental_parsing_tests {
    use super::*;
    use crate::parser::{parse_nix_uri, parse_params};

    #[test]
    fn parse_platform() {
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
    #[test]
    fn parse_basic_owner_rep() {
        let input = "owner/repo";
        let (rest, mut iter) = GitForge::parse_owner_repo_ref(input).unwrap();
        assert_eq!(rest, "");
        assert_eq!(Some("owner"), iter.next());
        assert_eq!(Some("repo"), iter.next());
        assert_eq!(None, iter.next());
    }
    #[test]
    fn parse_owner_repo_param_terminated() {
        let input = "owner/repo?foo=bar";
        let (rest, mut iter) = GitForge::parse_owner_repo_ref(input).unwrap();
        assert_eq!(rest, "?foo=bar");
        assert_eq!(Some("owner"), iter.next());
        assert_eq!(Some("repo"), iter.next());
        assert_eq!(None, iter.next());
    }
    #[test]
    fn parse_owner_repo_attr_terminated() {
        let input = "owner/repo#fizz.bar";
        let (rest, mut iter) = GitForge::parse_owner_repo_ref(input).unwrap();
        assert_eq!(rest, "#fizz.bar");
        assert_eq!(Some("owner"), iter.next());
        assert_eq!(Some("repo"), iter.next());
        assert_eq!(None, iter.next());
    }
    #[test]
    fn parse_owner_repo_rev_param_terminated() {
        let input = "owner/repo/rev?foo=bar";
        let (rest, mut iter) = GitForge::parse_owner_repo_ref(input).unwrap();
        assert_eq!(rest, "?foo=bar");
        assert_eq!(Some("owner"), iter.next());
        assert_eq!(Some("repo"), iter.next());
        assert_eq!(Some("rev"), iter.next());
        assert_eq!(None, iter.next());
    }
    #[test]
    fn parse_owner_repo_rev_attr_terminated() {
        let input = "owner/repo/rev#fizz.bar";
        let (rest, mut iter) = GitForge::parse_owner_repo_ref(input).unwrap();
        assert_eq!(rest, "#fizz.bar");
        assert_eq!(Some("owner"), iter.next());
        assert_eq!(Some("repo"), iter.next());
        assert_eq!(Some("rev"), iter.next());
        assert_eq!(None, iter.next());
    }
}
