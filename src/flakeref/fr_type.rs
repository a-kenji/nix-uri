use std::{
    fmt::Display,
    path::{Path, PathBuf},
};

use nom::{
    branch::alt,
    bytes::complete::{tag, take_till, take_until},
    combinator::{cond, map, opt, peek, rest, verify},
    IResult,
};
use serde::{Deserialize, Serialize};

use crate::{
    error::{NixUriError, NixUriResult},
    flakeref::forge::GitForge,
    parser::parse_url_type,
};

use super::{GitForgePlatform, TransportLayer};
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum FlakeRefType {
    // In URL form, the schema must be file+http://, file+https:// or file+file://. If the extension doesn’t correspond to a known archive format (as defined by the tarball fetcher), then the file+ prefix can be dropped.
    File {
        url: PathBuf,
    },
    //TODO: #155
    /// Git repositories. The location of the repository is specified by the attribute
    /// `url`. The `ref` arrribute defaults to resolving the `HEAD` reference.
    /// The `rev` attribute must exist in the branch or tag specified by `ref`, defaults
    /// to `ref`.
    Git {
        url: String,
        r#type: TransportLayer,
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
        r#type: TransportLayer,
    },
    /// Path must be a directory in the filesystem containing a `flake.nix`.
    /// Path must be an absolute path.
    Path {
        path: String,
    },
    Tarball {
        url: String,
        r#type: TransportLayer,
    },
    #[default]
    None,
}
impl FlakeRefType {
    // TODO: #158
    pub fn parse_file(input: &str) -> IResult<&str, Self> {
        alt((
            map(
                alt((
                    Self::parse_explicit_file_scheme,
                    Self::parse_http_file_scheme,
                )),
                |path| Self::File {
                    url: PathBuf::from(path),
                },
            ),
            map(Self::parse_naked, |path| Self::Path {
                path: format!("{}", path.display()),
            }),
        ))(input)
    }
    pub fn parse_naked(input: &str) -> IResult<&str, &Path> {
        // Check if input starts with `.` or `/`
        let (is_path, _) = peek(alt((tag("."), tag("/"))))(input)?;
        let (rest, path_str) = Self::path_parser(is_path)?;
        Ok((rest, Path::new(path_str)))
    }
    pub fn path_parser(input: &str) -> IResult<&str, &str> {
        verify(take_till(|c| c == '#' || c == '?'), |c: &str| {
            Path::new(c).is_absolute() && !c.contains("[") && !c.contains("]")
        })(input)
    }
    pub fn parse_explicit_file_scheme(input: &str) -> IResult<&str, &Path> {
        let (rest, _) = alt((tag("path:"), tag("file://"), tag("file+file://")))(input)?;
        let (rest, path_str) = Self::path_parser(rest)?;
        Ok((rest, Path::new(path_str)))
    }
    pub fn parse_http_file_scheme(input: &str) -> IResult<&str, &Path> {
        let (rest, _) = alt((tag("file+http://"), tag("file+https://")))(input)?;
        eprintln!("`file+http[s]://` not pet implemented");
        Err(nom::Err::Failure(nom::error::Error {
            input,
            code: nom::error::ErrorKind::Fail,
        }))
    }
    /// TODO: different platforms have different rules about the owner/repo/ref/ref strings. These
    /// rules are not checked for in the current form of the parser
    /// <github | gitlab | sourcehut>:<owner>/<repo>[/<rev | ref>]...
    pub fn parse_git_forge(input: &str) -> IResult<&str, Self> {
        map(GitForge::parse, Self::GitForge)(input)
    }
    /// <git | hg>[+<url-type]://
    pub fn parse_vc(input: &str) -> IResult<&str, Self> {
        alt((Self::parse_git_vc, Self::parse_hg_vc))(input)
    }
    pub fn parse_git_vc(input: &str) -> IResult<&str, Self> {
        let (path, tag) = alt((
            tag("git://"),
            tag("git+http://"),
            tag("git+https://"),
            tag("git+ssh://"),
            tag("git+file://"),
        ))(input)?;
        // TODO: un-yuck this trim-abomination
        let tag = tag.trim_end_matches("://");
        let tag = tag.trim_start_matches("git");
        let tag = tag.trim_start_matches("+");

        let tp = if tag.is_empty() {
            TransportLayer::File
        } else {
            TransportLayer::try_from(tag).unwrap()
        };

        let (rest, url) = take_till(|c| c == '#' || c == '?')(path)?;

        Ok((
            rest,
            Self::Git {
                r#type: tp,
                url: url.to_string(),
            },
        ))
    }
    pub fn parse_hg_vc(input: &str) -> IResult<&str, Self> {
        let (path, tag) = alt((
            tag("hg://"),
            tag("hg+http://"),
            tag("hg+https://"),
            tag("hg+ssh://"),
            tag("hg+file://"),
        ))(input)?;
        // TODO: un-yuck this trim-abomination
        let tag = tag.trim_end_matches("://");
        let tag = tag.trim_start_matches("hg");
        let tag = tag.trim_start_matches("+");

        let tp = if tag.is_empty() {
            TransportLayer::File
        } else {
            TransportLayer::try_from(tag).unwrap()
        };
        let (rest, url) = take_till(|c| c == '#' || c == '?')(path)?;
        Ok((
            rest,
            Self::Mercurial {
                r#type: tp,
                url: url.to_string(),
            },
        ))
    }
    pub fn parse(input: &str) -> IResult<&str, Self> {
        alt((Self::parse_git_forge, Self::parse_file, Self::parse_vc))(input)
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
                    // TODO: #158
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
                    // TODO: #162
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
            // TODO: #158
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
            // TODO: #158
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
            // TODO: #158
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
                if let TransportLayer::None = r#type {
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
            FlakeRefType::Mercurial { url, r#type } => {
                if let TransportLayer::None = r#type {
                    return write!(f, "hg:{url}");
                }
                let uri = format!("hg+{}:{url}", r#type);
                write!(f, "{uri}")
            }
            FlakeRefType::Path { path } => write!(f, "{}", path),
            // TODO: alternate tarball representation
            FlakeRefType::Tarball { url, r#type } => {
                write!(f, "file:{url}")
            }
            FlakeRefType::None => todo!(),
        }
    }
}

#[cfg(test)]
mod inc_parse_vc {
    use super::*;
    #[test]
    fn git_file() {
        let uri = "git:///foo/bar";
        let file_uri = "git+file:///foo/bar";
        let (rest, parsed_refpath) = FlakeRefType::parse(uri).unwrap();
        let (rest, file_parsed_refpath) = FlakeRefType::parse(file_uri).unwrap();
        assert_eq!(parsed_refpath, file_parsed_refpath);
        let expected_refpath = FlakeRefType::Git {
            url: "/foo/bar".to_string(),
            r#type: TransportLayer::File,
        };
        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }
    #[test]
    fn git_http() {
        let uri = "git+http:///foo/bar";
        let (rest, parsed_refpath) = FlakeRefType::parse(uri).unwrap();
        let expected_refpath = FlakeRefType::Git {
            url: "/foo/bar".to_string(),
            r#type: TransportLayer::Http,
        };
        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }
    #[test]
    fn git_https() {
        let uri = "git+https:///foo/bar";
        let (rest, parsed_refpath) = FlakeRefType::parse(uri).unwrap();
        let expected_refpath = FlakeRefType::Git {
            url: "/foo/bar".to_string(),
            r#type: TransportLayer::Https,
        };
        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }
    #[test]
    fn hg_file() {
        let uri = "hg:///foo/bar";
        let file_uri = "hg+file:///foo/bar";
        let (rest, parsed_refpath) = FlakeRefType::parse(uri).unwrap();
        let (rest, file_parsed_refpath) = FlakeRefType::parse(file_uri).unwrap();
        assert_eq!(file_parsed_refpath, parsed_refpath);
        let expected_refpath = FlakeRefType::Mercurial {
            url: "/foo/bar".to_string(),
            r#type: TransportLayer::File,
        };
        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }
    #[test]
    fn hg_http() {
        let uri = "hg+http:///foo/bar";
        let (rest, parsed_refpath) = FlakeRefType::parse(uri).unwrap();
        let expected_refpath = FlakeRefType::Mercurial {
            url: "/foo/bar".to_string(),
            r#type: TransportLayer::Http,
        };
        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }
    #[test]
    fn hg_https() {
        let uri = "hg+https:///foo/bar";
        let (rest, parsed_refpath) = FlakeRefType::parse(uri).unwrap();
        let expected_refpath = FlakeRefType::Mercurial {
            url: "/foo/bar".to_string(),
            r#type: TransportLayer::Https,
        };
        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }
}
#[cfg(test)]
mod inc_parse_file {
    use super::*;
    #[test]
    fn path_leader() {
        let uri = "path:/foo/bar";
        let (rest, parsed_refpath) = FlakeRefType::parse_file(uri).unwrap();
        let expected_refpath = FlakeRefType::File {
            url: PathBuf::from("/foo/bar"),
        };
        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }
    #[test]
    fn naked_abs() {
        let uri = "/foo/bar";
        let (rest, parsed_refpath) = FlakeRefType::parse_file(uri).unwrap();
        let expected_refpath = FlakeRefType::Path {
            path: "/foo/bar".to_string(),
        };
        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }
    #[test]
    #[ignore = "We don't yet handle relative paths"]
    fn naked_cwd() {
        let uri = "./foo/bar";
        let (rest, parsed_refpath) = FlakeRefType::parse_file(uri).unwrap();
        let expected_refpath = FlakeRefType::Path {
            path: "./foo/bar".to_string(),
        };
        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }
    #[test]
    #[ignore = "need to implement http location parsing"]
    fn http_layer() {
        let uri = "file+http://???";
        let (rest, parsed_refpath) = FlakeRefType::parse_file(uri).unwrap();
        let expected_refpath = FlakeRefType::File {
            url: PathBuf::from("/foo/bar"),
        };
        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }
    #[test]
    #[ignore = "need to implement https location parsing"]
    fn https_layer() {
        let uri = "file+https://???";
        let (rest, parsed_refpath) = FlakeRefType::parse_file(uri).unwrap();
        let expected_refpath = FlakeRefType::File {
            url: PathBuf::from("/foo/bar"),
        };
        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }
    #[test]
    fn file_layer() {
        let uri = "file+file:///foo/bar";
        let (rest, parsed_refpath) = FlakeRefType::parse_file(uri).unwrap();
        let expected_refpath = FlakeRefType::File {
            url: PathBuf::from("/foo/bar"),
        };
        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }
    #[test]
    fn file_then_path() {
        let path_uri = "file:///wheres/wally";
        let path_uri2 = "file:///wheres/wally/";

        let (rest, parsed_ref) = FlakeRefType::parse_file(path_uri).unwrap();
        assert_eq!(rest, "");
        let (rest, parsed_ref2) = FlakeRefType::parse_file(path_uri2).unwrap();
        assert_eq!(rest, "");

        let expected_ref = FlakeRefType::File {
            url: PathBuf::from("/wheres/wally"),
        };
        assert_eq!(expected_ref, parsed_ref);
        assert_eq!(expected_ref, parsed_ref2);
    }
    #[test]
    fn empty_param_term() {
        let path_uri = "file:///wheres/wally?";
        let path_uri2 = "file:///wheres/wally/?";

        let (rest, parsed_file) = FlakeRefType::parse_file(path_uri).unwrap();
        assert_eq!(rest, "?");
        let (rest, parsed_file2) = FlakeRefType::parse_file(path_uri2).unwrap();
        assert_eq!(rest, "?");

        let expected_ref = FlakeRefType::File {
            url: PathBuf::from("/wheres/wally"),
        };
        assert_eq!(expected_ref, parsed_file);
        assert_eq!(expected_ref, parsed_file2);
    }
    #[test]
    fn param_term() {
        let path_uri = "file:///wheres/wally?foo=bar#fizz";
        let path_uri2 = "file:///wheres/wally/?foo=bar#fizz";

        let (rest, parsed_file) = FlakeRefType::parse_file(path_uri).unwrap();
        assert_eq!(rest, "?foo=bar#fizz");
        let (rest, parsed_file2) = FlakeRefType::parse_file(path_uri2).unwrap();
        assert_eq!(rest, "?foo=bar#fizz");

        let expected_ref = FlakeRefType::File {
            url: PathBuf::from("/wheres/wally"),
        };
        assert_eq!(expected_ref, parsed_file);
        assert_eq!(expected_ref, parsed_file2);
    }
    #[test]
    fn empty_param_attr_term() {
        let path_uri = "file:///wheres/wally?#";
        let path_uri2 = "file:///wheres/wally/?#";

        let (rest, parsed_file) = FlakeRefType::parse_file(path_uri).unwrap();
        assert_eq!(rest, "?#");
        let (rest, parsed_file2) = FlakeRefType::parse_file(path_uri2).unwrap();
        assert_eq!(rest, "?#");

        let expected_ref = FlakeRefType::File {
            url: PathBuf::from("/wheres/wally"),
        };
        assert_eq!(expected_ref, parsed_file);
        assert_eq!(expected_ref, parsed_file2);

        let path_uri = "file:///wheres/wally#?";
        let path_uri2 = "file:///wheres/wally/#?";

        let (rest, parsed_file) = FlakeRefType::parse_file(path_uri).unwrap();
        assert_eq!(rest, "#?");
        let (rest, parsed_file2) = FlakeRefType::parse_file(path_uri2).unwrap();
        assert_eq!(rest, "#?");

        let expected_ref = FlakeRefType::File {
            url: PathBuf::from("/wheres/wally"),
        };
        assert_eq!(expected_ref, parsed_file);
        assert_eq!(expected_ref, parsed_file2);
    }
    #[test]
    fn attr_term() {
        let path_uri = "file:///wheres/wally#";
        let path_uri2 = "file:///wheres/wally/#";

        let (rest, parsed_file) = FlakeRefType::parse_file(path_uri).unwrap();
        assert_eq!(rest, "#");
        let (rest, parsed_file2) = FlakeRefType::parse_file(path_uri2).unwrap();
        assert_eq!(rest, "#");

        let expected_ref = FlakeRefType::File {
            url: PathBuf::from("/wheres/wally"),
        };
        assert_eq!(expected_ref, parsed_file);
        assert_eq!(expected_ref, parsed_file2);
        assert_eq!(rest, "#");
    }
}
