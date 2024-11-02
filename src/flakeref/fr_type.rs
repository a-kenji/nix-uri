use std::{
    fmt::Display,
    path::{Path, PathBuf},
};

use nom::{
    branch::alt,
    bytes::complete::{tag, take_till, take_until},
    combinator::{map, opt, rest, verify},
    IResult,
};
use serde::{Deserialize, Serialize};

use crate::{
    error::{NixUriError, NixUriResult},
    flakeref::forge::GitForge,
    parser::parse_url_type,
};

use super::{GitForgePlatform, UrlType};
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum FlakeRefType {
    // In URL form, the schema must be file+http://, file+https:// or file+file://. If the extension doesn’t correspond to a known archive format (as defined by the tarball fetcher), then the file+ prefix can be dropped.
    File {
        url: PathBuf,
    },
    /// Git repositories. The location of the repository is specified by the attribute
    /// `url`. The `ref` arrribute defaults to resolving the `HEAD` reference.
    /// The `rev` attribute must exist in the branch or tag specified by `ref`, defaults
    /// to `ref`.
    Git {
        url: String,
        r#type: UrlType,
    },

    GitForge(GitForge),
    Indirect {
        id: String,
        ref_or_rev: Option<String>,
    },
    // Matches `git` type, but schema is one of the following:
    // `hg+http`, `hg+https`, `hg+ssh` or `hg+file`.
    Mercurial {
        url: String,
        r#type: UrlType,
    },
    /// Path must be a directory in the filesystem containing a `flake.nix`.
    /// Path must be an absolute path.
    Path {
        path: String,
    },
    Tarball {
        url: String,
        r#type: UrlType,
    },
    #[default]
    None,
}
impl FlakeRefType {
    // TODO: parse string that leads with a file
    pub fn parse_file(input: &str) -> IResult<&str, &Path> {
        let (rest, _) = tag("file://")(input)?;
        let (rest, path_str) = verify(take_till(|c| c == '#' || c == '?'), |c: &str| {
            Path::new(c).is_absolute() && !c.contains("[") && !c.contains("]")
        })(rest)?;
        Ok((rest, Path::new(path_str)))
    }
    /// TODO: different platforms have different rules about the owner/repo/ref/ref strings. These
    /// rules are not checked for in the current form of the parser
    /// <github | gitlab | sourcehut>:<owner>/<repo>[/<rev | ref>]...
    pub fn parse_git_forge(input: &str) -> IResult<&str, Self> {
        map(GitForge::parse, Self::GitForge)(input)
    }
    /// <git | hg>[+<url-type]
    pub fn parse_vc(input: &str) -> IResult<&str, Self> {
        todo!("caution: `^git` is good for both `github:...` and `git[+...]://`");
    }
    pub fn parse(input: &str) -> IResult<&str, Self> {
        if let Ok((rest, res)) = Self::parse_git_forge(input) {
            return Ok((rest, res));
        } else if let Ok((rest, res)) = Self::parse_file(input) {
            let res = Self::File {
                url: res.to_path_buf(),
            };
            return Ok((rest, res));
        }

        todo!();
    }
    /// Parse type specific information, returns the [`FlakeRefType`]
    /// and the unparsed input
    pub fn parse_type(input: &str) -> NixUriResult<FlakeRefType> {
        use nom::sequence::separated_pair;
        let (_, maybe_explicit_type) = opt(separated_pair(
            take_until::<&str, &str, (&str, nom::error::ErrorKind)>(":"),
            tag(":"),
            rest,
        ))(input)?;
        if let Some((flake_ref_type_str, input)) = maybe_explicit_type {
            match flake_ref_type_str {
                "github" | "gitlab" | "sourcehut" => {
                    let (input, owner_and_repo_or_ref) = GitForge::parse_owner_repo_ref(input)?;
                    let mut parsed_iter = owner_and_repo_or_ref.map(|s| s.to_string());
                    let er_fn = |st: &str| {
                        NixUriError::MissingTypeParameter(flake_ref_type_str.into(), st.to_string())
                    };
                    let owner = parsed_iter.next().ok_or(er_fn("owner"))?;
                    let repo = parsed_iter.next().ok_or(er_fn("repo"))?;
                    let ref_or_rev = parsed_iter.next();
                    let platform = match flake_ref_type_str {
                        "github" => GitForgePlatform::GitHub,
                        "gitlab" => GitForgePlatform::GitLab,
                        "sourcehut" => GitForgePlatform::SourceHut,
                        _ => unreachable!(),
                    };
                    let res = FlakeRefType::GitForge(GitForge {
                        platform,
                        owner,
                        repo,
                        ref_or_rev,
                    });
                    Ok(res)
                }
                "path" => {
                    // TODO: check if path is an absolute path, if not error
                    let path = Path::new(input);
                    // TODO: make this check configurable for cli usage
                    if !path.is_absolute() || input.contains(']') || input.contains('[') {
                        return Err(NixUriError::NotAbsolute(input.into()));
                    }
                    if input.contains('#') || input.contains('?') {
                        return Err(NixUriError::PathCharacter(input.into()));
                    }
                    let flake_ref_type = FlakeRefType::Path { path: input.into() };
                    Ok(flake_ref_type)
                }

                _ => {
                    if flake_ref_type_str.starts_with("git+") {
                        let url_type = parse_url_type(flake_ref_type_str)?;
                        let (input, _tag) =
                            opt(tag::<&str, &str, (&str, nom::error::ErrorKind)>("//"))(input)?;
                        let flake_ref_type = FlakeRefType::Git {
                            url: input.into(),
                            r#type: url_type,
                        };
                        Ok(flake_ref_type)
                    } else if flake_ref_type_str.starts_with("hg+") {
                        let url_type = parse_url_type(flake_ref_type_str)?;
                        let (input, _tag) =
                            tag::<&str, &str, (&str, nom::error::ErrorKind)>("//")(input)?;
                        let flake_ref_type = FlakeRefType::Mercurial {
                            url: input.into(),
                            r#type: url_type,
                        };
                        Ok(flake_ref_type)
                    } else {
                        Err(NixUriError::UnknownUriType(flake_ref_type_str.into()))
                    }
                }
            }
        } else {
            // Implicit types can be paths, indirect flake_refs, or uri's.
            if input.starts_with('/') || input == "." {
                let flake_ref_type = FlakeRefType::Path { path: input.into() };
                let path = Path::new(input);
                // TODO: make this check configurable for cli usage
                if !path.is_absolute()
                    || input.contains(']')
                    || input.contains('[')
                    || !input.is_ascii()
                {
                    return Err(NixUriError::NotAbsolute(input.into()));
                }
                if input.contains('#') || input.contains('?') {
                    return Err(NixUriError::PathCharacter(input.into()));
                }
                return Ok(flake_ref_type);
            }
            //TODO: parse uri
            let (input, mut owner_and_repo_or_ref) = GitForge::parse_owner_repo_ref(input)?;
            if let Some(id) = owner_and_repo_or_ref.next() {
                if !id
                    .chars()
                    .all(|c| c.is_ascii_alphabetic() || c.is_control())
                    || id.is_empty()
                {
                    return Err(NixUriError::InvalidUrl(input.into()));
                }
                let flake_ref_type = FlakeRefType::Indirect {
                    id: id.to_string(),
                    ref_or_rev: owner_and_repo_or_ref.next().map(|s| s.to_string()),
                };
                Ok(flake_ref_type)
            } else {
                let (_input, mut owner_and_repo_or_ref) = GitForge::parse_owner_repo_ref(input)?;
                let id = if let Some(id) = owner_and_repo_or_ref.next() {
                    id
                } else {
                    input
                };
                if !id.chars().all(|c| c.is_ascii_alphabetic()) || id.is_empty() {
                    return Err(NixUriError::InvalidUrl(input.into()));
                }
                Ok(FlakeRefType::Indirect {
                    id: id.to_string(),
                    ref_or_rev: owner_and_repo_or_ref.next().map(|s| s.to_string()),
                })
            }
        }
    }
    /// Extract a common identifier from it's [`FlakeRefType`] variant.
    pub(crate) fn get_id(&self) -> Option<String> {
        match self {
            FlakeRefType::GitForge(GitForge { repo, .. }) => Some(repo.to_string()),
            FlakeRefType::File { .. }
            | FlakeRefType::Git { .. }
            | FlakeRefType::Tarball { .. }
            | FlakeRefType::None
            | FlakeRefType::Indirect { .. }
            | FlakeRefType::Mercurial { .. }
            | FlakeRefType::Path { .. } => None,
        }
    }
    pub fn get_repo(&self) -> Option<String> {
        match self {
            FlakeRefType::GitForge(GitForge { repo, .. }) => Some(repo.into()),
            // TODO: return a proper error, if ref_or_rev is tried to be specified
            FlakeRefType::Mercurial { .. }
            | FlakeRefType::Path { .. }
            | FlakeRefType::Indirect { .. }
            | FlakeRefType::Tarball { .. }
            | FlakeRefType::File { .. }
            | FlakeRefType::Git { .. }
            | FlakeRefType::None => None,
        }
    }
    pub fn get_owner(&self) -> Option<String> {
        match self {
            FlakeRefType::GitForge(GitForge { owner, .. }) => Some(owner.into()),
            // TODO: return a proper error, if ref_or_rev is tried to be specified
            FlakeRefType::Mercurial { .. }
            | FlakeRefType::Path { .. }
            | FlakeRefType::Indirect { .. }
            | FlakeRefType::Tarball { .. }
            | FlakeRefType::File { .. }
            | FlakeRefType::Git { .. }
            | FlakeRefType::None => None,
        }
    }
    pub fn ref_or_rev(&mut self, ref_or_rev_alt: Option<String>) -> Result<(), NixUriError> {
        match self {
            FlakeRefType::GitForge(GitForge { ref_or_rev, .. })
            | FlakeRefType::Indirect { ref_or_rev, .. } => {
                *ref_or_rev = ref_or_rev_alt;
            }
            // TODO: return a proper error, if ref_or_rev is tried to be specified
            FlakeRefType::Mercurial { .. }
            | FlakeRefType::Path { .. }
            | FlakeRefType::Tarball { .. }
            | FlakeRefType::File { .. }
            | FlakeRefType::Git { .. }
            | FlakeRefType::None => todo!(),
        }
        Ok(())
    }
}

impl Display for FlakeRefType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlakeRefType::File { url } => write!(f, "file:{}", url.display()),
            FlakeRefType::Git { url, r#type } => {
                if let UrlType::None = r#type {
                    return write!(f, "git:{url}");
                }
                let uri = format!("git+{}:{url}", r#type);
                write!(f, "{uri}")
            }
            FlakeRefType::GitForge(GitForge {
                platform,
                owner,
                repo,
                ref_or_rev,
            }) => {
                write!(f, "{platform}:{owner}/{repo}");
                if let Some(ref_or_rev) = ref_or_rev {
                    write!(f, "/{ref_or_rev}");
                }
                Ok(())
            }
            FlakeRefType::Indirect { id, ref_or_rev } => {
                if let Some(ref_or_rev) = ref_or_rev {
                    write!(f, "{id}/{ref_or_rev}")
                } else {
                    write!(f, "{id}")
                }
            }
            FlakeRefType::Mercurial { url, r#type } => todo!(),
            FlakeRefType::Path { path } => todo!(),
            // TODO: alternate tarball representation
            FlakeRefType::Tarball { url, r#type } => {
                write!(f, "file:{url}")
            }
            FlakeRefType::None => todo!(),
        }
    }
}

#[cfg(test)]
mod inc_parse_path {
    use super::*;
    #[test]
    fn naked() {
        let uri = "/foo/bar";
        let (rest, parsed_refpath) = FlakeRefType::parse_file(uri).unwrap();
        let expected_refpath = PathBuf::from("/foo/bar");
        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }
    #[test]
    fn plain() {
        let path_uri = "file:///wheres/wally";
        let path_uri2 = "file:///wheres/wally/";

        let (rest, parsed_file) = FlakeRefType::parse_file(path_uri).unwrap();
        assert_eq!(rest, "");
        let (rest, parsed_file2) = FlakeRefType::parse_file(path_uri2).unwrap();
        assert_eq!(rest, "");

        let expected_file = PathBuf::from("/wheres/wally");
        assert_eq!(expected_file, parsed_file);
        assert_eq!(expected_file, parsed_file2);
    }
    #[test]
    fn with_param() {
        let path_uri = "file:///wheres/wally?foo=bar#fizz";
        let path_uri2 = "file:///wheres/wally/?foo=bar#fizz";

        let (rest, parsed_file) = FlakeRefType::parse_file(path_uri).unwrap();
        assert_eq!(rest, "?foo=bar#fizz");
        let (rest, parsed_file2) = FlakeRefType::parse_file(path_uri2).unwrap();
        assert_eq!(rest, "?foo=bar#fizz");

        let expected_file = PathBuf::from("/wheres/wally");
        assert_eq!(expected_file, parsed_file);
        assert_eq!(expected_file, parsed_file2);
    }
    #[test]
    fn empty_param_attr() {
        let path_uri = "file:///wheres/wally?#";
        let path_uri2 = "file:///wheres/wally/?#";

        let (rest, parsed_file) = FlakeRefType::parse_file(path_uri).unwrap();
        assert_eq!(rest, "?#");
        let (rest, parsed_file2) = FlakeRefType::parse_file(path_uri2).unwrap();
        assert_eq!(rest, "?#");

        let expected_file = PathBuf::from("/wheres/wally");
        assert_eq!(expected_file, parsed_file);
        assert_eq!(expected_file, parsed_file2);

        let path_uri = "file:///wheres/wally#?";
        let path_uri2 = "file:///wheres/wally/#?";

        let (rest, parsed_file) = FlakeRefType::parse_file(path_uri).unwrap();
        assert_eq!(rest, "#?");
        let (rest, parsed_file2) = FlakeRefType::parse_file(path_uri2).unwrap();
        assert_eq!(rest, "#?");

        let expected_file = PathBuf::from("/wheres/wally");
        assert_eq!(expected_file, parsed_file);
        assert_eq!(expected_file, parsed_file2);
    }
    #[test]
    fn full_file_empty_attr() {
        let path_uri = "file:///wheres/wally#";
        let path_uri2 = "file:///wheres/wally/#";

        let (rest, parsed_file) = FlakeRefType::parse_file(path_uri).unwrap();
        assert_eq!(rest, "#");
        let (rest, parsed_file2) = FlakeRefType::parse_file(path_uri2).unwrap();
        assert_eq!(rest, "#");

        let expected_file = PathBuf::from("/wheres/wally");
        assert_eq!(rest, "#");
        assert_eq!(expected_file, parsed_file);
        assert_eq!(expected_file, parsed_file2);
    }
    #[test]
    fn full_file_empty_param() {
        let path_uri = "file:///wheres/wally?";
        let path_uri2 = "file:///wheres/wally/?";

        let (rest, parsed_file) = FlakeRefType::parse_file(path_uri).unwrap();
        assert_eq!(rest, "?");
        let (rest, parsed_file2) = FlakeRefType::parse_file(path_uri2).unwrap();
        assert_eq!(rest, "?");

        let expected_file = PathBuf::from("/wheres/wally");
        assert_eq!(expected_file, parsed_file);
        assert_eq!(expected_file, parsed_file2);
    }
}
