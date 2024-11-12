use std::{fmt::Display, path::Path};

use nom::{
    branch::alt,
    bytes::complete::{tag, take_till, take_until},
    character::complete::char,
    combinator::{map, opt, peek, rest, verify},
    error::{VerboseError, VerboseErrorKind},
    sequence::{preceded, separated_pair},
    Finish, IResult,
};
use serde::{Deserialize, Serialize};

use crate::{
    error::{NixUriError, NixUriResult},
    flakeref::forge::GitForge,
    parser::parse_transport_type,
};

use super::{
    resource_url::{ResourceType, ResourceUrl},
    GitForgePlatform,
};
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum FlakeRefType {
    Resource(ResourceUrl),

    GitForge(GitForge),
    Indirect {
        id: String,
        ref_or_rev: Option<String>,
    },
    /// Path must be a directory in the filesystem containing a `flake.nix`.
    /// Path must be an absolute path.
    Path {
        path: String,
    },
    #[default]
    None,
}

impl FlakeRefType {
    pub fn parse_path(input: &str) -> IResult<&str, Self, VerboseError<&str>> {
        let path_map = map(Self::path_parser, |path_str| Self::Path {
            path: path_str.to_string(),
        });
        preceded(opt(alt((tag("path://"), tag("path:")))), path_map)(input)
    }

    // TODO: #158
    pub fn parse_file(input: &str) -> IResult<&str, Self, VerboseError<&str>> {
        alt((
            map(
                alt((
                    // file+file
                    Self::parse_explicit_file_scheme,
                    // file+http(s)
                    Self::parse_http_file_scheme,
                )),
                // file
                |path| {
                    Self::Resource(ResourceUrl {
                        res_type: ResourceType::File,
                        location: path.display().to_string(),
                        transport_type: None,
                    })
                },
            ),
            map(Self::parse_naked, |path| Self::Path {
                path: format!("{}", path.display()),
            }),
        ))(input)
    }
    pub fn parse_naked(input: &str) -> IResult<&str, &Path, VerboseError<&str>> {
        // Check if input starts with `.` or `/`
        let (is_path, _) = peek(alt((char('.'), char('/'))))(input)?;
        let (rest, path_str) = Self::path_parser(is_path)?;
        Ok((rest, Path::new(path_str)))
    }
    pub fn path_parser(input: &str) -> IResult<&str, &str, VerboseError<&str>> {
        verify(take_till(|c| c == '#' || c == '?'), |c: &str| {
            Path::new(c).is_absolute() && !c.contains('[') && !c.contains(']')
        })(input)
    }
    pub fn parse_explicit_file_scheme(input: &str) -> IResult<&str, &Path, VerboseError<&str>> {
        let (rest, _) = alt((
            tag("file://"),
            tag("file+file://"),
            tag("file:"),
            tag("file+file:"),
        ))(input)?;
        let (rest, path_str) = Self::path_parser(rest)?;
        Ok((rest, Path::new(path_str)))
    }
    pub fn parse_http_file_scheme(input: &str) -> IResult<&str, &Path, VerboseError<&str>> {
        let (_rest, _) = alt((tag("file+http://"), tag("file+https://")))(input)?;
        eprintln!("`file+http[s]://` not pet implemented");
        Err(nom::Err::Failure(VerboseError {
            errors: vec![(input, VerboseErrorKind::Context("http[s] not implemented"))],
        }))
    }
    /// TODO: different platforms have different rules about the owner/repo/ref/ref strings. These
    /// rules are not checked for in the current form of the parser
    /// <github | gitlab | sourcehut>:<owner>/<repo>[/<rev | ref>]...
    pub fn parse_git_forge(input: &str) -> IResult<&str, Self, VerboseError<&str>> {
        map(GitForge::parse, Self::GitForge)(input)
    }
    /// <git | hg>[+<transport-type]://
    pub fn parse_resource(input: &str) -> IResult<&str, Self, VerboseError<&str>> {
        map(ResourceUrl::parse, Self::Resource)(input)
    }
    pub fn parse(input: &str) -> IResult<&str, Self, VerboseError<&str>> {
        alt((
            Self::parse_path,
            Self::parse_git_forge,
            Self::parse_file,
            Self::parse_resource,
        ))(input)
    }
    /// Parse type specific information, returns the [`FlakeRefType`]
    /// and the unparsed input
    pub fn parse_type(input: &str) -> NixUriResult<Self> {
        let (_, maybe_explicit_type) = opt(separated_pair(
            take_until::<&str, &str, VerboseError<&str>>(":"),
            char(':'),
            rest,
        ))(input)
        .finish()?;
        if let Some((flake_ref_type_str, input)) = maybe_explicit_type {
            match flake_ref_type_str {
                "github" | "gitlab" | "sourcehut" => {
                    let (_input, owner_and_repo_or_ref) =
                        GitForge::parse_owner_repo_ref(input).finish()?;
                    // TODO: #158
                    let _er_fn = |st: &str| {
                        NixUriError::MissingTypeParameter(flake_ref_type_str.into(), st.to_string())
                    };
                    let owner = owner_and_repo_or_ref.0.to_string();
                    let repo = owner_and_repo_or_ref.1.to_string();
                    let ref_or_rev = owner_and_repo_or_ref.2.map(str::to_string);
                    let platform = match flake_ref_type_str {
                        "github" => GitForgePlatform::GitHub,
                        "gitlab" => GitForgePlatform::GitLab,
                        "sourcehut" => GitForgePlatform::SourceHut,
                        _ => unreachable!(),
                    };
                    let res = Self::GitForge(GitForge {
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
                    let flake_ref_type = Self::Path { path: input.into() };
                    Ok(flake_ref_type)
                }

                _ => {
                    if flake_ref_type_str.starts_with("git+") {
                        let transport_type = parse_transport_type(flake_ref_type_str)?;
                        let (input, _tag) =
                            opt(tag::<&str, &str, VerboseError<&str>>("//"))(input).finish()?;
                        let flake_ref_type = Self::Resource(ResourceUrl {
                            res_type: ResourceType::Git,
                            location: input.into(),
                            transport_type: Some(transport_type),
                        });
                        Ok(flake_ref_type)
                    } else if flake_ref_type_str.starts_with("hg+") {
                        let transport_type = parse_transport_type(flake_ref_type_str)?;
                        let (input, _tag) =
                            tag::<&str, &str, VerboseError<&str>>("//")(input).finish()?;
                        let flake_ref_type = Self::Resource(ResourceUrl {
                            res_type: ResourceType::Mercurial,
                            location: input.into(),
                            transport_type: Some(transport_type),
                        });
                        Ok(flake_ref_type)
                    } else {
                        Err(NixUriError::UnknownUriType(flake_ref_type_str.into()))
                    }
                }
            }
        } else {
            // Implicit types can be paths, indirect flake_refs, or uri's.
            if input.starts_with('/') || input == "." {
                let flake_ref_type = Self::Path { path: input.into() };
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

            let (input, owner_and_repo_or_ref) = GitForge::parse_owner_repo_ref(input).finish()?;
            // Comments left in for reference. We are in the process of moving error context
            // generation into the parser itself, as opposed to up here. The GitForge parser used
            // here will have to take on responsibility of contextualising failures;
            // if let Some(id) = owner_and_repo_or_ref {
            if !owner_and_repo_or_ref
                .0
                .chars()
                .all(|c| c.is_ascii_alphabetic() || c.is_control())
                || owner_and_repo_or_ref.0.is_empty()
            {
                return Err(NixUriError::InvalidUrl(input.into()));
            }
            let flake_ref_type = Self::Indirect {
                id: owner_and_repo_or_ref.0.to_string(),
                ref_or_rev: owner_and_repo_or_ref.2.map(str::to_string),
            };
            Ok(flake_ref_type)
            // } else {
            //     let (_input, mut owner_and_repo_or_ref) = GitForge::parse_owner_repo_ref(input)?;
            //     let id = if let Some(id) = owner_and_repo_or_ref.next() {
            //         id
            //     } else {
            //         input
            //     };
            //     if !id.chars().all(|c| c.is_ascii_alphabetic()) || id.is_empty() {
            //         return Err(NixUriError::InvalidUrl(input.into()));
            //     }
            //     Ok(FlakeRefType::Indirect {
            //         id: id.to_string(),
            //         ref_or_rev: owner_and_repo_or_ref.next().map(|s| s.to_string()),
            //     })
            // }
        }
    }
    /// Extract a common identifier from it's [`FlakeRefType`] variant.
    pub(crate) fn get_id(&self) -> Option<String> {
        match self {
            Self::GitForge(GitForge { repo, .. }) => Some(repo.to_string()),
            _ => None,
        }
    }
    pub fn get_repo(&self) -> Option<String> {
        match self {
            Self::GitForge(GitForge { repo, .. }) => Some(repo.into()),
            // TODO: #158
            _ => None,
        }
    }
    pub fn get_owner(&self) -> Option<String> {
        match self {
            Self::GitForge(GitForge { owner, .. }) => Some(owner.into()),
            // TODO: #158
            _ => None,
        }
    }
    pub fn ref_or_rev(&mut self, ref_or_rev_alt: Option<String>) -> Result<(), NixUriError> {
        match self {
            Self::GitForge(GitForge { ref_or_rev, .. }) | Self::Indirect { ref_or_rev, .. } => {
                *ref_or_rev = ref_or_rev_alt;
            }
            // TODO: #158
            _ => {
                return Err(NixUriError::UnsupportedByType(
                    "ref_or_rev".to_string(),
                    "git-forge types && indirect types".to_string(),
                ))
            }
        }
        Ok(())
    }
}

impl Display for FlakeRefType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // TODO: alternate tarball representation
            Self::Resource(ResourceUrl {
                res_type,
                location,
                transport_type,
            }) => {
                write!(f, "{}", res_type)?;
                if let Some(transport_type) = transport_type {
                    write!(f, "+{}", transport_type)?;
                }
                write!(f, "://{}", location)
            }
            Self::GitForge(GitForge {
                platform,
                owner,
                repo,
                ref_or_rev,
            }) => {
                write!(f, "{platform}:{owner}/{repo}")?;
                if let Some(ref_or_rev) = ref_or_rev {
                    write!(f, "/{ref_or_rev}")?;
                }
                Ok(())
            }
            Self::Indirect { id, ref_or_rev } => {
                if let Some(ref_or_rev) = ref_or_rev {
                    write!(f, "{id}/{ref_or_rev}")
                } else {
                    write!(f, "{id}")
                }
            }
            Self::Path { path } => write!(f, "{}", path),
            Self::None => todo!(),
        }
    }
}

#[cfg(test)]
mod inc_parse_vc {
    use crate::TransportLayer;

    use super::*;

    #[test]
    fn parse_git_github_collision() {
        let hub = "github:foo/bar";
        let git = "git:///foo/bar";
        let (rest_hub, parsed_hub) = FlakeRefType::parse(hub).unwrap();
        let (rest_git, parsed_git) = FlakeRefType::parse(git).unwrap();
        let expected_hub = FlakeRefType::GitForge(GitForge {
            platform: GitForgePlatform::GitHub,
            owner: "foo".to_string(),
            repo: "bar".to_string(),
            ref_or_rev: None,
        });
        let expected_git = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Git,
            location: "/foo/bar".to_string(),
            transport_type: None,
        });

        assert_eq!("", rest_hub);
        assert_eq!("", rest_git);
        assert_eq!(expected_git, parsed_git);
        assert_eq!(expected_hub, parsed_hub);
    }

    #[test]
    fn git_file() {
        let uri = "git:///foo/bar";
        let file_uri = "git+file:///foo/bar";

        let expected_refpath = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Git,
            location: "/foo/bar".to_string(),
            transport_type: None,
        });
        let expected_filerefpath = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Git,
            location: "/foo/bar".to_string(),
            transport_type: Some(TransportLayer::File),
        });

        let (rest, parsed_refpath) = FlakeRefType::parse(uri).unwrap();
        assert!(rest.is_empty());
        let (rest, file_parsed_refpath) = FlakeRefType::parse(file_uri).unwrap();
        assert!(rest.is_empty());

        assert_eq!(expected_refpath, parsed_refpath);
        assert_eq!(expected_filerefpath, file_parsed_refpath);
    }

    #[test]
    fn git_http() {
        let uri = "git+http:///foo/bar";
        let expected_refpath = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Git,
            location: "/foo/bar".to_string(),
            transport_type: Some(TransportLayer::Http),
        });

        let (rest, parsed_refpath) = FlakeRefType::parse(uri).unwrap();

        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }

    #[test]
    fn git_https() {
        let uri = "git+https:///foo/bar";
        let expected_refpath = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Git,
            location: "/foo/bar".to_string(),
            transport_type: Some(TransportLayer::Https),
        });

        let (rest, parsed_refpath) = FlakeRefType::parse(uri).unwrap();

        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }

    #[test]
    fn hg_file() {
        let uri = "hg:///foo/bar";
        let file_uri = "hg+file:///foo/bar";
        let expected_refpath = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Mercurial,
            location: "/foo/bar".to_string(),
            transport_type: None,
        });
        let file_expected_refpath = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Mercurial,
            location: "/foo/bar".to_string(),
            transport_type: Some(TransportLayer::File),
        });

        let (rest, parsed_refpath) = FlakeRefType::parse(uri).unwrap();
        assert!(rest.is_empty());
        let (rest, file_parsed_refpath) = FlakeRefType::parse(file_uri).unwrap();

        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
        assert_eq!(file_expected_refpath, file_parsed_refpath);
    }

    #[test]
    fn hg_http() {
        let uri = "hg+http:///foo/bar";
        let expected_refpath = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Mercurial,
            location: "/foo/bar".to_string(),
            transport_type: Some(TransportLayer::Http),
        });

        let (rest, parsed_refpath) = FlakeRefType::parse(uri).unwrap();

        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }

    #[test]
    fn hg_https() {
        let uri = "hg+https:///foo/bar";
        let expected_refpath = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Mercurial,
            location: "/foo/bar".to_string(),
            transport_type: Some(TransportLayer::Https),
        });

        let (rest, parsed_refpath) = FlakeRefType::parse(uri).unwrap();

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
        let expected_refpath = FlakeRefType::Path {
            path: "/foo/bar".to_string(),
        };

        let (rest, parsed_refpath) = FlakeRefType::parse(uri).unwrap();

        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }

    #[test]
    fn naked_abs() {
        let uri = "/foo/bar";
        let expected_refpath = FlakeRefType::Path {
            path: "/foo/bar".to_string(),
        };

        let (rest, parsed_refpath) = FlakeRefType::parse(uri).unwrap();

        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }

    #[test]
    #[ignore = "We don't yet handle relative paths"]
    fn naked_cwd() {
        let uri = "./foo/bar";
        let expected_refpath = FlakeRefType::Path {
            path: "./foo/bar".to_string(),
        };

        let (rest, parsed_refpath) = FlakeRefType::parse_file(uri).unwrap();

        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }

    #[test]
    #[ignore = "need to implement http location parsing"]
    fn http_layer() {
        let uri = "file+http://???";
        let expected_refpath = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::File,
            location: "/foo/bar".to_string(),
            transport_type: None,
        });

        let (rest, parsed_refpath) = FlakeRefType::parse_file(uri).unwrap();

        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }

    #[test]
    #[ignore = "need to implement https location parsing"]
    fn https_layer() {
        let uri = "file+https://???";
        let expected_refpath = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::File,
            location: "/foo/bar".to_string(),
            transport_type: None,
        });

        let (rest, parsed_refpath) = FlakeRefType::parse_file(uri).unwrap();

        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }

    #[test]
    fn file_layer() {
        let uri = "file+file:///foo/bar";
        let expected_refpath = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::File,
            location: "/foo/bar".to_string(),
            transport_type: None,
        });

        let (rest, parsed_refpath) = FlakeRefType::parse_file(uri).unwrap();

        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }

    #[test]
    fn file_then_path() {
        let path_uri = "file:///wheres/wally";
        let path_uri2 = "file:///wheres/wally/";

        let mut expected_ref = ResourceUrl {
            res_type: ResourceType::File,
            location: "/wheres/wally".to_string(),
            transport_type: None,
        };

        let (rest, parsed_ref) = FlakeRefType::parse_file(path_uri).unwrap();
        assert_eq!(rest, "");
        let (rest, parsed_ref2) = FlakeRefType::parse_file(path_uri2).unwrap();

        assert_eq!(rest, "");
        assert_eq!(FlakeRefType::Resource(expected_ref.clone()), parsed_ref);
        expected_ref.location = "/wheres/wally/".to_string();
        assert_eq!(FlakeRefType::Resource(expected_ref), parsed_ref2);
    }

    #[test]
    fn empty_param_term() {
        let path_uri = "file:///wheres/wally?";
        let path_uri2 = "file:///wheres/wally/?";

        let mut expected_ref = ResourceUrl {
            res_type: ResourceType::File,
            location: "/wheres/wally".to_string(),
            transport_type: None,
        };

        let (rest, parsed_file) = FlakeRefType::parse_file(path_uri).unwrap();
        assert_eq!(rest, "?");
        let (rest, parsed_file2) = FlakeRefType::parse_file(path_uri2).unwrap();

        assert_eq!(rest, "?");
        assert_eq!(FlakeRefType::Resource(expected_ref.clone()), parsed_file);
        expected_ref.location = "/wheres/wally/".to_string();
        assert_eq!(FlakeRefType::Resource(expected_ref), parsed_file2);
    }

    #[test]
    fn param_term() {
        let path_uri = "file:///wheres/wally?foo=bar#fizz";
        let path_uri2 = "file:///wheres/wally/?foo=bar#fizz";

        let (rest, parsed_file) = FlakeRefType::parse_file(path_uri).unwrap();
        assert_eq!(rest, "?foo=bar#fizz");
        let (rest, parsed_file2) = FlakeRefType::parse_file(path_uri2).unwrap();
        assert_eq!(rest, "?foo=bar#fizz");

        let mut expected_ref = ResourceUrl {
            res_type: ResourceType::File,
            location: "/wheres/wally".to_string(),
            transport_type: None,
        };
        assert_eq!(FlakeRefType::Resource(expected_ref.clone()), parsed_file);
        expected_ref.location = "/wheres/wally/".to_string();
        assert_eq!(FlakeRefType::Resource(expected_ref), parsed_file2);
    }

    #[test]
    fn empty_param_attr_term() {
        let path_uri = "file:///wheres/wally?#";
        let path_uri2 = "file:///wheres/wally/?#";

        let (rest, parsed_file) = FlakeRefType::parse_file(path_uri).unwrap();
        assert_eq!(rest, "?#");
        let (rest, parsed_file2) = FlakeRefType::parse_file(path_uri2).unwrap();
        assert_eq!(rest, "?#");

        let mut expected_ref = ResourceUrl {
            res_type: ResourceType::File,
            location: "/wheres/wally".to_string(),
            transport_type: None,
        };
        assert_eq!(FlakeRefType::Resource(expected_ref.clone()), parsed_file);
        expected_ref.location = "/wheres/wally/".to_string();
        assert_eq!(FlakeRefType::Resource(expected_ref.clone()), parsed_file2);

        let path_uri = "file:///wheres/wally#?";
        let path_uri2 = "file:///wheres/wally/#?";

        let (rest, parsed_file) = FlakeRefType::parse_file(path_uri).unwrap();
        assert_eq!(rest, "#?");
        let (rest, parsed_file2) = FlakeRefType::parse_file(path_uri2).unwrap();
        assert_eq!(rest, "#?");

        expected_ref.location = "/wheres/wally".to_string();
        assert_eq!(FlakeRefType::Resource(expected_ref.clone()), parsed_file);
        expected_ref.location = "/wheres/wally/".to_string();
        assert_eq!(FlakeRefType::Resource(expected_ref), parsed_file2);
    }

    #[test]
    fn attr_term() {
        let path_uri = "file:///wheres/wally#";
        let path_uri2 = "file:///wheres/wally/#";

        let (rest, parsed_file) = FlakeRefType::parse_file(path_uri).unwrap();
        assert_eq!(rest, "#");
        let (rest, parsed_file2) = FlakeRefType::parse_file(path_uri2).unwrap();
        assert_eq!(rest, "#");

        let mut expected_ref = ResourceUrl {
            res_type: ResourceType::File,
            location: "/wheres/wally".to_string(),
            transport_type: None,
        };
        assert_eq!(FlakeRefType::Resource(expected_ref.clone()), parsed_file);
        expected_ref.location = "/wheres/wally/".to_string();
        assert_eq!(FlakeRefType::Resource(expected_ref), parsed_file2);
        assert_eq!(rest, "#");
    }
}
