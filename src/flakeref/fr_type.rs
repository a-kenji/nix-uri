use std::{fmt::Display, path::Path};

use nom::{
    Finish, IResult,
    branch::alt,
    bytes::complete::{take_till, take_until},
    character::complete::char,
    combinator::{map, opt, peek, rest, verify},
    error::context,
    sequence::{preceded, separated_pair, terminated},
};
use nom_supreme::tag::complete::tag;
use serde::{Deserialize, Serialize};

use crate::{
    IErr,
    error::{NixUriError, NixUriResult},
    flakeref::{TransportLayer, forge::GitForge},
    parser::parse_transport_type,
};

use super::{
    GitForgePlatform,
    resource_url::{ResourceType, ResourceUrl},
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
    pub fn parse_path(input: &str) -> IResult<&str, Self, IErr<&str>> {
        let path_map = map(Self::path_parser, |path_str| Self::Path {
            path: path_str.to_string(),
        });
        preceded(opt(alt((tag("path://"), tag("path:")))), path_map)(input)
    }

    // TODO: #158
    pub fn parse_file(input: &str) -> IResult<&str, Self, IErr<&str>> {
        context(
            "path resource",
            alt((
                // Handle file+http[s]:// as Resource with proper transport
                Self::parse_file_with_http_transport,
                map(
                    alt((
                        // file+file
                        Self::parse_explicit_file_scheme,
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
            )),
        )(input)
    }
    pub fn parse_naked(input: &str) -> IResult<&str, &Path, IErr<&str>> {
        // Check if input starts with `.` or `/`
        let (is_path, _) = peek(context("path location", alt((char('.'), char('/')))))(input)?;
        let (rest, path_str) = Self::path_parser(is_path)?;
        Ok((rest, Path::new(path_str)))
    }
    pub fn path_parser(input: &str) -> IResult<&str, &str, IErr<&str>> {
        preceded(peek(alt((char('.'), char('/')))), Self::path_verifier)(input)
    }
    pub fn path_verifier(input: &str) -> IResult<&str, &str, IErr<&str>> {
        context(
            "path validation",
            verify(take_till(|c| c == '#' || c == '?'), |c: &str| {
                !c.contains('[') && !c.contains(']')
            }),
        )(input)
    }
    pub fn parse_explicit_file_scheme(input: &str) -> IResult<&str, &Path, IErr<&str>> {
        let (rest, _) = context(
            "file resource",
            preceded(
                tag("file"),
                preceded(opt(tag("+file")), terminated(char(':'), opt(tag("//")))),
            ),
        )(input)?;
        let (rest, path_str) = Self::path_parser(rest)?;
        Ok((rest, Path::new(path_str)))
    }
    pub fn parse_file_with_http_transport(input: &str) -> IResult<&str, Self, IErr<&str>> {
        use nom::bytes::complete::take_till;

        let (rest, scheme) = alt((tag("file+https"), tag("file+http")))(input)?;
        let (rest, _) = tag("://")(rest)?;
        let (rest, location) = take_till(|c| c == '#' || c == '?')(rest)?;

        let transport_type = match scheme {
            "file+https" => Some(TransportLayer::Https),
            "file+http" => Some(TransportLayer::Http),
            _ => unreachable!(),
        };

        Ok((
            rest,
            Self::Resource(ResourceUrl {
                res_type: ResourceType::File,
                location: location.to_string(),
                transport_type,
            }),
        ))
    }

    pub fn parse_http_file_scheme(input: &str) -> IResult<&str, &Path, IErr<&str>> {
        use nom::bytes::complete::take_till;

        let (rest, _) = context(
            "networked file",
            preceded(tag("file+http"), alt((tag("://"), tag("s://")))),
        )(input)?;

        // Take everything until # or ? (parameters/fragments)
        let (rest, location) = take_till(|c| c == '#' || c == '?')(rest)?;

        // For file+http[s], we don't return a Path but need to be handled differently
        // This method signature is wrong for this use case, but we need to work with existing code
        // Return a fake path that will be handled properly by the parent
        Ok((rest, Path::new(location)))
    }
    /// TODO: different platforms have different rules about the owner/repo/ref/ref strings. These
    /// rules are not checked for in the current form of the parser
    /// <github | gitlab | sourcehut>:<owner>/<repo>[/<rev | ref>]...
    pub fn parse_git_forge(input: &str) -> IResult<&str, Self, IErr<&str>> {
        map(GitForge::parse, Self::GitForge)(input)
    }
    /// <git | hg>[+<transport-type]://
    pub fn parse_resource(input: &str) -> IResult<&str, Self, IErr<&str>> {
        map(ResourceUrl::parse, Self::Resource)(input)
    }
    /// Parse plain HTTP/HTTPS URLs with auto-detection
    pub fn parse_plain_url(input: &str) -> IResult<&str, Self, IErr<&str>> {
        use crate::parser::is_tarball;

        let (rest, scheme) = alt((tag("https"), tag("http")))(input)?;
        let (rest, _) = tag("://")(rest)?;
        let (rest, location) = context("url location", take_till(|c| c == '#' || c == '?'))(rest)?;

        let res_type = if is_tarball(location) {
            ResourceType::Tarball
        } else {
            ResourceType::File
        };

        let transport_type = match scheme {
            "https" => Some(TransportLayer::Https),
            "http" => Some(TransportLayer::Http),
            _ => None,
        };

        Ok((
            rest,
            Self::Resource(ResourceUrl {
                res_type,
                location: location.to_string(),
                transport_type,
            }),
        ))
    }
    /// Parse indirect flake references (flake:id[/ref] or bare id[/ref])
    pub fn parse_indirect(input: &str) -> IResult<&str, Self, IErr<&str>> {
        use nom::bytes::complete::{tag, take_till, take_while1};
        use nom::combinator::{opt, verify};
        use nom::sequence::preceded;

        // Try explicit flake: scheme first
        if let Ok((rest, _)) = tag::<&str, &str, IErr<&str>>("flake:")(input) {
            let (rest, id) = verify(
                take_while1(|c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                |s: &str| !s.is_empty() && s.chars().next().unwrap().is_ascii_alphabetic(),
            )(rest)?;

            let (rest, ref_or_rev) =
                opt(preceded(char('/'), take_till(|c| c == '#' || c == '?')))(rest)?;

            return Ok((
                rest,
                Self::Indirect {
                    id: id.to_string(),
                    ref_or_rev: ref_or_rev.map(str::to_string),
                },
            ));
        }

        // Try bare flake ID (id[/ref])
        // Must be alphanumeric with hyphens/underscores, can't contain protocols or paths
        if !input.contains("://") && !input.starts_with('/') && !input.starts_with('.') {
            let slash_count = input.matches('/').count();

            // Only allow simple patterns: "id" or "id/ref"
            if slash_count <= 1 {
                let (rest, id) = verify(
                    take_while1(|c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                    |s: &str| !s.is_empty() && s.chars().next().unwrap().is_ascii_alphabetic(),
                )(input)?;

                let (rest, ref_or_rev) =
                    opt(preceded(char('/'), take_till(|c| c == '#' || c == '?')))(rest)?;

                return Ok((
                    rest,
                    Self::Indirect {
                        id: id.to_string(),
                        ref_or_rev: ref_or_rev.map(str::to_string),
                    },
                ));
            }
        }

        Err(nom::Err::Error(IErr::Base {
            location: input,
            kind: nom_supreme::error::BaseErrorKind::Kind(nom::error::ErrorKind::Fail),
        }))
    }

    pub fn parse(input: &str) -> IResult<&str, Self, IErr<&str>> {
        alt((
            context("raw path", Self::parse_path),
            context("git forge", Self::parse_git_forge),
            context("file", Self::parse_file),
            context("plain url", Self::parse_plain_url),
            context("resource", Self::parse_resource),
            context("indirect", Self::parse_indirect),
        ))(input)
    }
    /// Parse type specific information, returns the [`FlakeRefType`]
    /// and the unparsed input
    pub fn parse_type(input: &str) -> NixUriResult<Self> {
        let (_, maybe_explicit_type) = opt(separated_pair(
            take_until::<&str, &str, IErr<&str>>(":"),
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
                "flake" => {
                    // Parse flake:id[/ref] as indirect reference
                    let (id, ref_or_rev) = if let Some(pos) = input.find('/') {
                        let (id_part, ref_part) = input.split_at(pos);
                        (id_part, Some(&ref_part[1..])) // Skip the '/'
                    } else {
                        (input, None)
                    };

                    // Validate flake ID format
                    if id.is_empty() || !id.chars().next().unwrap().is_ascii_alphabetic() {
                        return Err(NixUriError::InvalidUrl(input.into()));
                    }

                    if !id
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                    {
                        return Err(NixUriError::InvalidUrl(input.into()));
                    }

                    let flake_ref_type = Self::Indirect {
                        id: id.to_string(),
                        ref_or_rev: ref_or_rev.map(str::to_string),
                    };
                    Ok(flake_ref_type)
                }

                _ => {
                    if flake_ref_type_str.starts_with("git+") {
                        let transport_type = parse_transport_type(flake_ref_type_str)?;
                        let (input, _tag) =
                            opt(tag::<&str, &str, IErr<&str>>("//"))(input).finish()?;
                        let flake_ref_type = Self::Resource(ResourceUrl {
                            res_type: ResourceType::Git,
                            location: input.into(),
                            transport_type: Some(transport_type),
                        });
                        Ok(flake_ref_type)
                    } else if flake_ref_type_str.starts_with("hg+") {
                        let transport_type = parse_transport_type(flake_ref_type_str)?;
                        let (input, _tag) = tag::<&str, &str, IErr<&str>>("//")(input).finish()?;
                        let flake_ref_type = Self::Resource(ResourceUrl {
                            res_type: ResourceType::Mercurial,
                            location: input.into(),
                            transport_type: Some(transport_type),
                        });
                        Ok(flake_ref_type)
                    } else if flake_ref_type_str.starts_with("tarball+") {
                        let transport_type = parse_transport_type(flake_ref_type_str)?;
                        let (input, _tag) =
                            opt(tag::<&str, &str, IErr<&str>>("//"))(input).finish()?;
                        let flake_ref_type = Self::Resource(ResourceUrl {
                            res_type: ResourceType::Tarball,
                            location: input.into(),
                            transport_type: Some(transport_type),
                        });
                        Ok(flake_ref_type)
                    } else if flake_ref_type_str.starts_with("file+") {
                        let transport_type = parse_transport_type(flake_ref_type_str)?;
                        let (input, _tag) =
                            opt(tag::<&str, &str, IErr<&str>>("//"))(input).finish()?;
                        let flake_ref_type = Self::Resource(ResourceUrl {
                            res_type: ResourceType::File,
                            location: input.into(),
                            transport_type: Some(transport_type),
                        });
                        Ok(flake_ref_type)
                    } else if flake_ref_type_str == "https" || flake_ref_type_str == "http" {
                        // Plain HTTP/HTTPS URL - auto-detect type based on extension
                        use crate::parser::is_tarball;

                        let (input, _tag) = tag::<&str, &str, IErr<&str>>("//")(input).finish()?;
                        let res_type = if is_tarball(input) {
                            ResourceType::Tarball
                        } else {
                            ResourceType::File
                        };
                        let transport_type = match flake_ref_type_str {
                            "https" => Some(TransportLayer::Https),
                            "http" => Some(TransportLayer::Http),
                            _ => None,
                        };

                        let flake_ref_type = Self::Resource(ResourceUrl {
                            res_type,
                            location: input.into(),
                            transport_type,
                        });
                        Ok(flake_ref_type)
                    } else if flake_ref_type_str == "git" {
                        // Bare git:// protocol
                        let (input, _tag) = tag::<&str, &str, IErr<&str>>("//")(input).finish()?;
                        let flake_ref_type = Self::Resource(ResourceUrl {
                            res_type: ResourceType::Git,
                            location: input.into(),
                            transport_type: None, // Native git protocol, no transport layer
                        });
                        Ok(flake_ref_type)
                    } else {
                        Err(NixUriError::UnknownUriType(flake_ref_type_str.into()))
                    }
                }
            }
        } else {
            // Implicit types can be paths, indirect flake_refs, or uri's.
            if input.starts_with('/')
                || input.starts_with("./")
                || input.starts_with("../")
                || input == "."
                || input == ".."
            {
                let flake_ref_type = Self::Path { path: input.into() };
                // Check for invalid characters but allow both absolute and relative paths
                if input.contains(']') || input.contains('[') || !input.is_ascii() {
                    return Err(NixUriError::InvalidUrl(input.into()));
                }
                if input.contains('#') || input.contains('?') {
                    return Err(NixUriError::PathCharacter(input.into()));
                }
                return Ok(flake_ref_type);
            }

            // Check if it looks like a bare flake ID first (single identifier, optionally followed by /ref)
            let slash_count = input.matches('/').count();
            if slash_count <= 1 {
                // Try as bare flake ID with optional ref
                let (id, ref_or_rev) = if let Some(pos) = input.find('/') {
                    let (id_part, ref_part) = input.split_at(pos);
                    (id_part, Some(&ref_part[1..])) // Skip the '/'
                } else {
                    (input, None)
                };

                // Validate flake ID format (must start with letter, can contain alphanumeric, hyphens, underscores)
                if !id.is_empty()
                    && id.chars().next().unwrap().is_ascii_alphabetic()
                    && id
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                {
                    let flake_ref_type = Self::Indirect {
                        id: id.to_string(),
                        ref_or_rev: ref_or_rev.map(str::to_string),
                    };
                    return Ok(flake_ref_type);
                }
            }

            // Try to parse as git forge pattern (owner/repo[/ref]) if not a simple flake ID
            if let Ok((rest, owner_and_repo_or_ref)) =
                GitForge::parse_owner_repo_ref(input).finish()
            {
                if !owner_and_repo_or_ref
                    .0
                    .chars()
                    .all(|c| c.is_ascii_alphabetic() || c.is_control())
                    || owner_and_repo_or_ref.0.is_empty()
                {
                    return Err(NixUriError::InvalidUrl(rest.into()));
                }
                let flake_ref_type = Self::Indirect {
                    id: owner_and_repo_or_ref.0.to_string(),
                    ref_or_rev: owner_and_repo_or_ref.2.map(str::to_string),
                };
                return Ok(flake_ref_type);
            }

            // Fallback error
            Err(NixUriError::InvalidUrl(input.into()))
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
                ));
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

    #[test]
    fn tarball_https_transport() {
        let uri = "tarball+https://example.com/file.tar.gz";
        let expected = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Tarball,
            location: "example.com/file.tar.gz".to_string(),
            transport_type: Some(TransportLayer::Https),
        });

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn tarball_http_transport() {
        let uri = "tarball+http://example.com/file.zip";
        let expected = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Tarball,
            location: "example.com/file.zip".to_string(),
            transport_type: Some(TransportLayer::Http),
        });

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn file_https_transport() {
        let uri = "file+https://example.com/file.txt";
        let expected = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::File,
            location: "example.com/file.txt".to_string(),
            transport_type: Some(TransportLayer::Https),
        });

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn file_http_transport() {
        let uri = "file+http://example.com/file.txt";
        let expected = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::File,
            location: "example.com/file.txt".to_string(),
            transport_type: Some(TransportLayer::Http),
        });

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn bare_git_protocol() {
        let uri = "git://github.com/user/repo.git";
        let expected = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Git,
            location: "github.com/user/repo.git".to_string(),
            transport_type: None,
        });

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn plain_https_tarball_autodetect() {
        let uri = "https://example.com/file.tar.gz";
        let expected = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Tarball,
            location: "example.com/file.tar.gz".to_string(),
            transport_type: Some(TransportLayer::Https),
        });

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn plain_https_file_autodetect() {
        let uri = "https://example.com/file.txt";
        let expected = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::File,
            location: "example.com/file.txt".to_string(),
            transport_type: Some(TransportLayer::Https),
        });

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn plain_http_tarball_autodetect() {
        let uri = "http://example.com/archive.zip";
        let expected = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Tarball,
            location: "example.com/archive.zip".to_string(),
            transport_type: Some(TransportLayer::Http),
        });

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn plain_http_file_autodetect() {
        let uri = "http://example.com/README.md";
        let expected = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::File,
            location: "example.com/README.md".to_string(),
            transport_type: Some(TransportLayer::Http),
        });

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn different_tarball_extensions() {
        let test_cases = vec![
            "https://example.com/file.tar.gz",
            "https://example.com/file.tar.bz2",
            "https://example.com/file.tar.xz",
            "https://example.com/file.tgz",
            "https://example.com/file.zip",
        ];

        for uri in test_cases {
            let result = FlakeRefType::parse_type(uri).unwrap();
            match result {
                FlakeRefType::Resource(ResourceUrl {
                    res_type: ResourceType::Tarball,
                    ..
                }) => {
                    // Expected
                }
                _ => panic!("Expected tarball for URI: {}", uri),
            }
        }
    }

    #[test]
    fn case_sensitive_extensions() {
        let uri_lowercase = "https://example.com/file.tar.gz";
        let result_lowercase = FlakeRefType::parse_type(uri_lowercase).unwrap();
        match result_lowercase {
            FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::Tarball,
                ..
            }) => {
                // Expected
            }
            _ => panic!("Expected tarball for lowercase extension"),
        }

        // Uppercase extension should be treated as File, not Tarball
        let uri_uppercase = "https://example.com/file.TAR.GZ";
        let result_uppercase = FlakeRefType::parse_type(uri_uppercase).unwrap();
        match result_uppercase {
            FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::File,
                ..
            }) => {
                // Expected
            }
            _ => panic!("Expected file for uppercase extension"),
        }
    }
}

#[cfg(test)]
mod inc_parse_indirect {
    use super::*;

    #[test]
    fn flake_explicit_scheme_simple() {
        let uri = "flake:nixpkgs";
        let expected = FlakeRefType::Indirect {
            id: "nixpkgs".to_string(),
            ref_or_rev: None,
        };

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn flake_explicit_scheme_with_ref() {
        let uri = "flake:nixpkgs/release-23.05";
        let expected = FlakeRefType::Indirect {
            id: "nixpkgs".to_string(),
            ref_or_rev: Some("release-23.05".to_string()),
        };

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn flake_explicit_scheme_with_hyphens() {
        let uri = "flake:my-flake";
        let expected = FlakeRefType::Indirect {
            id: "my-flake".to_string(),
            ref_or_rev: None,
        };

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn flake_explicit_scheme_with_underscores() {
        let uri = "flake:my_flake";
        let expected = FlakeRefType::Indirect {
            id: "my_flake".to_string(),
            ref_or_rev: None,
        };

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn flake_explicit_scheme_invalid_start_with_number() {
        let uri = "flake:123invalid";
        let result = FlakeRefType::parse_type(uri);
        assert!(result.is_err());
    }

    #[test]
    fn flake_explicit_scheme_empty() {
        let uri = "flake:";
        let result = FlakeRefType::parse_type(uri);
        assert!(result.is_err());
    }

    #[test]
    fn flake_explicit_scheme_invalid_characters() {
        let uri = "flake:invalid!";
        let result = FlakeRefType::parse_type(uri);
        assert!(result.is_err());
    }

    #[test]
    fn simple_flake_id() {
        let uri = "simple-flake";
        let expected = FlakeRefType::Indirect {
            id: "simple-flake".to_string(),
            ref_or_rev: None,
        };

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn flake_id_with_underscores() {
        let uri = "flake_with_underscores";
        let expected = FlakeRefType::Indirect {
            id: "flake_with_underscores".to_string(),
            ref_or_rev: None,
        };

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn flake_id_two_part() {
        let uri = "nixpkgs/unstable";
        let expected = FlakeRefType::Indirect {
            id: "nixpkgs".to_string(),
            ref_or_rev: Some("unstable".to_string()),
        };

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn bare_flake_id_with_numbers() {
        let uri = "nixpkgs23";
        let expected = FlakeRefType::Indirect {
            id: "nixpkgs23".to_string(),
            ref_or_rev: None,
        };

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);

        // Test via parse() method too
        let (rest, result) = FlakeRefType::parse(uri).unwrap();
        assert_eq!("", rest);
        assert_eq!(expected, result);
    }

    #[test]
    fn bare_flake_id_edge_cases() {
        // Test with too many slashes - should fail as indirect, no fallback should work
        let uri = "my-flake/branch/deep/reference";
        // This should fail because it has too many slashes - only id/ref is allowed for bare IDs
        let result = FlakeRefType::parse(uri);
        assert!(
            result.is_err(),
            "Multi-slash URIs should fail when not matching any scheme"
        );

        // Test single character ID
        let uri = "a";
        let expected = FlakeRefType::Indirect {
            id: "a".to_string(),
            ref_or_rev: None,
        };
        let (rest, result) = FlakeRefType::parse(uri).unwrap();
        assert_eq!("", rest);
        assert_eq!(expected, result);
    }

    #[test]
    fn flake_scheme_validation_edge_cases() {
        // Empty ID after flake:
        let uri = "flake:";
        let result = FlakeRefType::parse_type(uri);
        assert!(result.is_err());

        // ID starting with number
        let uri = "flake:123invalid";
        let result = FlakeRefType::parse_type(uri);
        assert!(result.is_err());

        // ID with invalid characters
        let uri = "flake:invalid!";
        let result = FlakeRefType::parse_type(uri);
        assert!(result.is_err());

        // Very long but valid ID
        let uri = "flake:very-long-flake-name-with-many-dashes-and_underscores_123";
        let expected = FlakeRefType::Indirect {
            id: "very-long-flake-name-with-many-dashes-and_underscores_123".to_string(),
            ref_or_rev: None,
        };
        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn protocol_collision_edge_cases() {
        // Ensure git:// doesn't collide with github:
        let git_uri = "git://example.com/repo.git";
        let github_uri = "github:user/repo";

        let (_, git_result) = FlakeRefType::parse(git_uri).unwrap();
        let (_, github_result) = FlakeRefType::parse(github_uri).unwrap();

        match git_result {
            FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::Git,
                ..
            }) => {
                // Expected
            }
            _ => panic!("Expected git resource for git:// URL"),
        }

        match github_result {
            FlakeRefType::GitForge(_) => {
                // Expected
            }
            _ => panic!("Expected git forge for github: URL"),
        }
    }

    #[test]
    fn http_https_autodetection_edge_cases() {
        let test_cases = vec![
            // Valid tarball extensions
            ("https://example.com/file.tar.gz", ResourceType::Tarball),
            ("https://example.com/file.tar.bz2", ResourceType::Tarball),
            ("https://example.com/file.tar.xz", ResourceType::Tarball),
            ("https://example.com/file.tar.zst", ResourceType::Tarball),
            ("https://example.com/file.tgz", ResourceType::Tarball),
            ("https://example.com/file.zip", ResourceType::Tarball),
            ("https://example.com/file.tar", ResourceType::Tarball),
            // Extensions that are NOT tarball (bare compression formats)
            ("https://example.com/file.gz", ResourceType::File),
            ("https://example.com/file.bz2", ResourceType::File),
            ("https://example.com/file.xz", ResourceType::File),
            // Other file types
            ("https://example.com/file.txt", ResourceType::File),
            ("https://example.com/README.md", ResourceType::File),
            ("https://example.com/file", ResourceType::File), // No extension
        ];

        for (uri, expected_type) in test_cases {
            let (_, result) = FlakeRefType::parse(uri).unwrap();
            match result {
                FlakeRefType::Resource(ResourceUrl { res_type, .. }) => {
                    assert_eq!(expected_type, res_type, "Failed for URI: {}", uri);
                }
                _ => panic!("Expected resource for URI: {}", uri),
            }
        }
    }

    #[test]
    fn transport_scheme_combinations() {
        // Test all transport combinations work
        let test_cases = vec![
            (
                "git+https://example.com/repo.git",
                ResourceType::Git,
                Some(TransportLayer::Https),
            ),
            (
                "git+http://example.com/repo.git",
                ResourceType::Git,
                Some(TransportLayer::Http),
            ),
            (
                "git+file://path/to/repo",
                ResourceType::Git,
                Some(TransportLayer::File),
            ),
            (
                "hg+https://example.com/repo",
                ResourceType::Mercurial,
                Some(TransportLayer::Https),
            ),
            (
                "hg+http://example.com/repo",
                ResourceType::Mercurial,
                Some(TransportLayer::Http),
            ),
            (
                "hg+file://path/to/repo",
                ResourceType::Mercurial,
                Some(TransportLayer::File),
            ),
            (
                "tarball+https://example.com/file.tar.gz",
                ResourceType::Tarball,
                Some(TransportLayer::Https),
            ),
            (
                "tarball+http://example.com/file.zip",
                ResourceType::Tarball,
                Some(TransportLayer::Http),
            ),
            (
                "file+https://example.com/file.txt",
                ResourceType::File,
                Some(TransportLayer::Https),
            ),
            (
                "file+http://example.com/file.txt",
                ResourceType::File,
                Some(TransportLayer::Http),
            ),
        ];

        for (uri, expected_res_type, expected_transport) in test_cases {
            let (_, result) = FlakeRefType::parse(uri).unwrap();
            match result {
                FlakeRefType::Resource(ResourceUrl {
                    res_type,
                    transport_type,
                    ..
                }) => {
                    assert_eq!(
                        expected_res_type, res_type,
                        "Resource type mismatch for: {}",
                        uri
                    );
                    assert_eq!(
                        expected_transport, transport_type,
                        "Transport type mismatch for: {}",
                        uri
                    );
                }
                _ => panic!("Expected resource for URI: {}", uri),
            }
        }
    }

    #[test]
    fn relative_path_edge_cases() {
        let test_cases = vec![
            "./",
            "../",
            "./path",
            "../path",
            "./path/to/flake",
            "../path/to/flake",
            "../../deeply/nested/path",
        ];

        for uri in test_cases {
            let (rest, result) = FlakeRefType::parse(uri).unwrap();
            assert_eq!("", rest, "Parse should consume entire input for: {}", uri);
            match result {
                FlakeRefType::Path { path } => {
                    assert_eq!(uri, path, "Path should match input for: {}", uri);
                }
                _ => panic!("Expected path for URI: {}", uri),
            }
        }
    }

    #[test]
    fn flake_id_complex_names() {
        let uri = "complex-flake/feature-branch";
        let expected = FlakeRefType::Indirect {
            id: "complex-flake".to_string(),
            ref_or_rev: Some("feature-branch".to_string()),
        };

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn flake_id_boundary_cases() {
        // Single character flake ID
        let uri = "a";
        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(
            result,
            FlakeRefType::Indirect {
                id: "a".to_string(),
                ref_or_rev: None
            }
        );

        // Flake ID with maximum allowed characters
        let uri = "abcDEF123-_";
        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(
            result,
            FlakeRefType::Indirect {
                id: "abcDEF123-_".to_string(),
                ref_or_rev: None
            }
        );
    }
}

#[cfg(test)]
mod inc_parse_errors {
    use super::*;

    #[test]
    fn error_unsupported_scheme() {
        let uri = "unsupported://example.com";
        let result = FlakeRefType::parse_type(uri);
        assert!(result.is_err());
    }

    #[test]
    fn error_malformed_url() {
        let uri = "://invalid";
        let result = FlakeRefType::parse_type(uri);
        assert!(result.is_err());
    }

    #[test]
    fn path_with_invalid_characters() {
        let uri = "/path/with[brackets]";
        let result = FlakeRefType::parse_type(uri);
        assert!(result.is_err());
    }

    #[test]
    fn path_with_query_fragment() {
        let uri = "/path/with?query#fragment";
        let result = FlakeRefType::parse_type(uri);
        assert!(result.is_err());
    }

    #[test]
    fn file_extension_edge_cases() {
        // File without extension should be treated as File, not Tarball
        let uri = "https://example.com/README";
        let result = FlakeRefType::parse_type(uri).unwrap();
        match result {
            FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::File,
                ..
            }) => {
                // Expected
            }
            _ => panic!("Expected file resource for extensionless file"),
        }
    }

    #[test]
    fn url_with_port() {
        let uri = "https://example.com:8080/file.tar.gz";
        let result = FlakeRefType::parse_type(uri).unwrap();
        match result {
            FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::Tarball,
                location,
                transport_type: Some(TransportLayer::Https),
            }) => {
                assert_eq!(location, "example.com:8080/file.tar.gz");
            }
            _ => panic!("Expected tarball resource with HTTPS transport"),
        }
    }

    #[test]
    fn mixed_case_domain() {
        let uri = "https://Example.COM/file.tar.gz";
        let result = FlakeRefType::parse_type(uri).unwrap();
        match result {
            FlakeRefType::Resource(ResourceUrl { location, .. }) => {
                assert_eq!(location, "Example.COM/file.tar.gz");
            }
            _ => panic!("Expected resource"),
        }
    }

    #[test]
    fn very_long_url() {
        let long_path = "a".repeat(1000);
        let uri = format!("https://example.com/{}.tar.gz", long_path);
        let result = FlakeRefType::parse_type(&uri);

        // Should parse successfully even with very long URLs
        assert!(result.is_ok());
    }

    #[test]
    fn transport_scheme_combinations() {
        // All valid combinations for tarball
        let valid_tarballs = vec![
            "tarball+https://example.com/file.tar.gz",
            "tarball+http://example.com/file.tar.gz",
            "tarball+file:///path/to/file.tar.gz",
        ];

        for uri in valid_tarballs {
            let result = FlakeRefType::parse_type(uri);
            assert!(result.is_ok(), "Failed to parse valid tarball URI: {}", uri);
        }

        // All valid combinations for file
        let valid_files = vec![
            "file+https://example.com/file.txt",
            "file+http://example.com/file.txt",
            "file+file:///path/to/file.txt",
        ];

        for uri in valid_files {
            let result = FlakeRefType::parse_type(uri);
            assert!(result.is_ok(), "Failed to parse valid file URI: {}", uri);
        }
    }

    #[test]
    fn real_world_github_archive() {
        let uri = "https://github.com/user/repo/archive/main.tar.gz";
        let expected = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Tarball,
            location: "github.com/user/repo/archive/main.tar.gz".to_string(),
            transport_type: Some(TransportLayer::Https),
        });

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
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
    fn relative_path_current_dir() {
        let uri = ".";
        let expected = FlakeRefType::Path {
            path: ".".to_string(),
        };

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn relative_path_parent_dir() {
        let uri = "..";
        let expected = FlakeRefType::Path {
            path: "..".to_string(),
        };

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn relative_path_current_subdir() {
        let uri = "./relative/path";
        let expected = FlakeRefType::Path {
            path: "./relative/path".to_string(),
        };

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn relative_path_parent_subdir() {
        let uri = "../parent/path";
        let expected = FlakeRefType::Path {
            path: "../parent/path".to_string(),
        };

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn complex_path_with_dots() {
        let uri = "./path/with/../../complex/structure";
        let expected = FlakeRefType::Path {
            path: "./path/with/../../complex/structure".to_string(),
        };

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
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
    fn http_layer() {
        let uri = "file+http://example.com/file.txt";
        let expected_refpath = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::File,
            location: "example.com/file.txt".to_string(),
            transport_type: Some(TransportLayer::Http),
        });

        let (rest, parsed_refpath) = FlakeRefType::parse_file(uri).unwrap();

        assert!(rest.is_empty());
        assert_eq!(expected_refpath, parsed_refpath);
    }

    #[test]
    fn https_layer() {
        let uri = "file+https://example.com/file.txt";
        let expected_refpath = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::File,
            location: "example.com/file.txt".to_string(),
            transport_type: Some(TransportLayer::Https),
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
