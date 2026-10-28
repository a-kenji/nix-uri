use std::fmt::Display;

use serde::{Deserialize, Serialize};

use crate::{
    error::{NixUriError, NixUriResult, ParseExpected, UnsupportedReason},
    flakeref::{
        RefLocation, TransportLayer,
        encoding::{decode_percent, is_url_path_char},
        forge::GitForge,
        validators::{is_sha1_hex, looks_like_rev, validated_ref_name},
    },
    parser::is_tarball,
};

use super::{
    GitForgePlatform,
    resource_url::{ResourceType, ResourceUrl},
};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum FlakeRefType {
    /// A resource-style URL (`git+`, `hg+`, `file+`, `tarball+`); see
    /// [`ResourceUrl`] for the typed shape.
    Resource(ResourceUrl),
    /// A git-forge shorthand (`github:`, `gitlab:`, `sourcehut:`); see
    /// [`GitForge`] for the typed shape.
    GitForge(GitForge),
    /// Indirect (registry) flake reference, e.g. `flake:nixpkgs` or
    /// `flake:nixpkgs/release-23.05`.
    ///
    /// Like [`GitForge`], `ref_` and `rev` are typed slots filled at parse
    /// time by inspecting the path-component value: 40-hex goes to `rev`,
    /// everything else to `ref_`. Nix's indirect form accepts up to three
    /// segments `flake:id/ref/rev`; when both are present, both slots
    /// populate.
    /// `location` records whether a present value would render as
    /// `flake:id/<value>` or `flake:id?ref=<value>`.
    Indirect {
        id: String,
        ref_: Option<String>,
        rev: Option<String>,
        location: RefLocation,
    },
    /// Path must be a directory in the filesystem containing a `flake.nix`.
    /// Path must be an absolute path.
    ///
    /// `rev` carries the optional 40-hex commit pin from a `?rev=` query
    /// parameter. Nix accepts `rev`, `narHash`, `revCount`, and
    /// `lastModified` on `path:` URLs; `narHash` and the counts ride on
    /// [`crate::LocationParameters`], but `rev` is a typed slot so locked
    /// store-path inputs round-trip without losing their pin. There is
    /// no path-component form for the rev (Path has no `/<rev>` shape in
    /// Nix's grammar), so it always renders as `?rev=`.
    ///
    /// `path:` URLs may use the empty-authority form (`path:///abs`,
    /// equivalent to `path:/abs`) -- Nix rejects only when the URL
    /// authority's host is non-empty, so `path://host/...` errors but
    /// `path:///abs` parses. To preserve the internal byte-for-byte
    /// round-trip the empty-authority form is stored verbatim (leading
    /// `//` kept on `path`); the slash-collapse normalisation Nix
    /// performs at Display time is intentionally deferred.
    Path { path: String, rev: Option<String> },
}

/// `Default` is only the seed for the in-progress `FlakeRef` inside
/// `parse_nix_uri`; an empty-path value is never round-trippable on its own
/// (the empty-input guard rejects it before it can escape).
impl Default for FlakeRefType {
    fn default() -> Self {
        Self::Path {
            path: String::new(),
            rev: None,
        }
    }
}

impl FlakeRefType {
    /// Parse the kind of a URL-shaped flake reference: `<scheme>:<rest>`
    /// with the query and fragment already split off. Flake ids and paths
    /// without a scheme are handled by the parser before and after this
    /// stage.
    pub(crate) fn parse_type(input: &str) -> NixUriResult<Self> {
        let Some((flake_ref_type_str, rest_input)) = input.split_once(':') else {
            return Err(NixUriError::InvalidUrl(input.into()));
        };
        match flake_ref_type_str {
            "github" | "gitlab" | "sourcehut" => {
                let platform = match flake_ref_type_str {
                    "github" => GitForgePlatform::GitHub,
                    "gitlab" => GitForgePlatform::GitLab,
                    _ => GitForgePlatform::SourceHut,
                };
                parse_git_forge(platform, rest_input).map(Self::GitForge)
            }
            "path" => {
                // `path://` is rejected only when the URL authority's
                // host is non-empty. An empty authority (`path:///abs`)
                // is the absolute path `/abs`. The body is stored
                // verbatim (leading slashes preserved) so Display
                // round-trips.
                if let Some(after) = rest_input.strip_prefix("//") {
                    let host_end = after.find('/').unwrap_or(after.len());
                    if !after[..host_end].is_empty() {
                        return Err(NixUriError::Unsupported(UnsupportedReason::Authority {
                            scheme: "path",
                        }));
                    }
                }
                check_url_path(rest_input)?;
                // An empty `path:` is the root directory.
                let path = if rest_input.is_empty() {
                    "/"
                } else {
                    rest_input
                };
                Ok(Self::Path {
                    path: path.into(),
                    rev: None,
                })
            }
            "flake" => {
                // Empty segments are skipped when splitting the URL
                // path, so `flake:nixpkgs//main` collapses to
                // `flake:nixpkgs/main`.
                let segments: Vec<&str> = rest_input.split('/').filter(|s| !s.is_empty()).collect();
                indirect_from_segments(&segments, rest_input)
            }
            _ => {
                let (res_type, transport_type, require_authority) = match flake_ref_type_str
                    .split_once('+')
                {
                    Some((application, transport)) => {
                        let res_type = match application {
                            "git" => ResourceType::Git,
                            "hg" => ResourceType::Mercurial,
                            "tarball" => ResourceType::Tarball,
                            "file" => ResourceType::File,
                            _ => return Err(unsupported_uri_type(flake_ref_type_str)),
                        };
                        let transport = TransportLayer::try_from(transport)?;
                        // Only Mercurial insists on `//`.
                        (res_type, Some(transport), application == "hg")
                    }
                    // Plain `http(s)://` and `file:` URLs are classified
                    // by their extension. A round-trip from the explicit
                    // `tarball+file://` shape (whose Display strips
                    // `tarball+`) lands back on the same kind. `file:`
                    // takes no authority, so `file:.` is accepted.
                    None => match flake_ref_type_str {
                        "https" => (by_extension(rest_input), Some(TransportLayer::Https), true),
                        "http" => (by_extension(rest_input), Some(TransportLayer::Http), true),
                        "file" => (by_extension(rest_input), Some(TransportLayer::File), false),
                        // Native git over the wire, no `+<transport>` layer.
                        "git" => (ResourceType::Git, None, true),
                        _ => return Err(unsupported_uri_type(flake_ref_type_str)),
                    },
                };
                let location = match rest_input.strip_prefix("//") {
                    Some(location) => location,
                    None if require_authority => {
                        return Err(NixUriError::Parse {
                            position: flake_ref_type_str.len() + 1,
                            expected: ParseExpected::Tag("//"),
                        });
                    }
                    None => rest_input,
                };
                check_authority(location, transport_type.as_ref())?;
                Ok(Self::Resource(ResourceUrl::new(
                    res_type,
                    location.into(),
                    transport_type,
                )))
            }
        }
    }

    /// Repository identifier for the kind: the `repo` of a `GitForge` or the
    /// trailing path segment of a `Resource(Git)` URL (with any `.git`
    /// suffix stripped). The public entry point is [`crate::FlakeRef::id`].
    pub(crate) fn id(&self) -> Option<&str> {
        match self {
            Self::GitForge(GitForge { repo, .. }) => Some(repo.as_str()),
            Self::Resource(ResourceUrl {
                res_type: ResourceType::Git,
                location,
                ..
            }) => {
                // Extract repo from "domain.com/owner/repo" or "domain.com/owner/repo.git".
                location
                    .split('/')
                    .nth(2)
                    .map(|s| s.strip_suffix(".git").unwrap_or(s))
            }
            _ => None,
        }
    }

    /// Repository name for the kind. The public entry point is
    /// [`crate::FlakeRef::repo`].
    pub(crate) fn repo(&self) -> Option<&str> {
        match self {
            Self::GitForge(GitForge { repo, .. }) => Some(repo.as_str()),
            Self::Resource(ResourceUrl {
                res_type: ResourceType::Git,
                location,
                ..
            }) => {
                // Parse "domain.com/owner/repo" or "domain.com/owner/repo.git".
                location
                    .split('/')
                    .nth(2)
                    .map(|s| s.strip_suffix(".git").unwrap_or(s))
            }
            _ => None,
        }
    }

    /// Owner (user/organisation) for the kind. The public entry point is
    /// [`crate::FlakeRef::owner`].
    pub(crate) fn owner(&self) -> Option<&str> {
        match self {
            Self::GitForge(GitForge { owner, .. }) => Some(owner.as_str()),
            Self::Resource(ResourceUrl {
                res_type: ResourceType::Git,
                location,
                ..
            }) => {
                // Parse "domain.com/owner/repo" -> "owner".
                location.split('/').nth(1)
            }
            _ => None,
        }
    }

    /// Domain (host) for the kind. Returns the canonical host string for
    /// git-forge platforms and the host portion of a `Resource(Git)` URL,
    /// retaining `:port` when the port is non-default for the scheme
    /// (mirrors HTTP-library `Authority` semantics; flake-edit consumes
    /// `domain()` directly as `api_host_for(domain)` input). The public
    /// entry point is [`crate::FlakeRef::domain`].
    pub(crate) fn domain(&self) -> Option<&str> {
        match self {
            Self::GitForge(GitForge { platform, .. }) => match platform {
                GitForgePlatform::GitHub => Some("github.com"),
                GitForgePlatform::GitLab => Some("gitlab.com"),
                GitForgePlatform::SourceHut => Some("git.sr.ht"),
            },
            Self::Resource(ResourceUrl {
                res_type: ResourceType::Git,
                location,
                transport_type,
                ..
            }) => {
                // URL form is `[user@]host[:port]/owner/repo`. The parser
                // only accepts a numeric (or empty) port. Strip any
                // leading `user@`, take everything before the first `/`,
                // and keep the port only when it is not the scheme's
                // default.
                let after_user = location
                    .split_once('@')
                    .map_or(location.as_str(), |(_, rest)| rest);
                let path_start = after_user.find('/').unwrap_or(after_user.len());
                let authority = &after_user[..path_start];
                if authority.is_empty() {
                    return None;
                }
                let Some((host, port_str)) = authority.split_once(':') else {
                    return Some(authority);
                };
                if host.is_empty() {
                    return None;
                }
                if port_str.is_empty() {
                    return Some(host);
                }
                let default_port = match transport_type {
                    Some(TransportLayer::Https) => Some("443"),
                    Some(TransportLayer::Http) => Some("80"),
                    Some(TransportLayer::Ssh) => Some("22"),
                    Some(TransportLayer::File) | None => None,
                };
                if default_port == Some(port_str) {
                    Some(host)
                } else {
                    Some(authority)
                }
            }
            _ => None,
        }
    }
    /// Whether this kind admits a `ref` per Nix's per-scheme attribute
    /// rules: yes for [`Self::GitForge`], [`Self::Indirect`], and
    /// [`Self::Resource`] of [`ResourceType::Git`] /
    /// [`ResourceType::Mercurial`]; no for [`Self::Path`] and
    /// `Resource(File | Tarball)`. Backs
    /// [`crate::FlakeRef::try_with_ref`]'s loud-failure decision; the
    /// silent-no-op [`Self::set_ref`] does not consult it.
    pub(crate) fn allows_ref(&self) -> bool {
        match self {
            Self::GitForge(_) | Self::Indirect { .. } => true,
            Self::Resource(res) => {
                matches!(res.res_type, ResourceType::Git | ResourceType::Mercurial)
            }
            Self::Path { .. } => false,
        }
    }

    /// Set the typed `ref_` slot. `Path` has no ref slot in Nix's grammar
    /// (only `rev`, `narHash`, `revCount`, `lastModified` are recognised
    /// on `path:`), so the Path arm is a no-op; callers that want to
    /// refuse `?ref=` on `path:` do so before reaching this method.
    pub(crate) fn set_ref(&mut self, new_ref: Option<String>) {
        match self {
            Self::GitForge(forge) => forge.ref_ = new_ref,
            Self::Indirect { ref_, .. } => *ref_ = new_ref,
            Self::Resource(res) => res.ref_ = new_ref,
            Self::Path { .. } => {}
        }
    }

    /// Set the typed `rev` slot. Path's slot has no path-component
    /// spelling in Nix's grammar and always renders as `?rev=` via the
    /// `FlakeRef` Display block.
    pub(crate) fn set_rev(&mut self, new_rev: Option<String>) {
        match self {
            Self::GitForge(forge) => forge.rev = new_rev,
            Self::Resource(res) => res.rev = new_rev,
            Self::Indirect { rev, .. } | Self::Path { rev, .. } => *rev = new_rev,
        }
    }

    /// Set the [`RefLocation`] on kinds that carry one. `Path` is a no-op:
    /// its rev has no path-component representation, so the routing tag
    /// is fixed to `QueryParameter` and not stored on the variant. The
    /// caller does not have to discriminate by kind.
    pub(crate) fn set_ref_location(&mut self, loc: RefLocation) {
        match self {
            Self::GitForge(forge) => forge.location = loc,
            Self::Indirect { location, .. } => *location = loc,
            Self::Resource(res) => res.ref_location = loc,
            Self::Path { .. } => {}
        }
    }
}

fn unsupported_uri_type(scheme: &str) -> NixUriError {
    NixUriError::Unsupported(UnsupportedReason::UriType { ty: scheme.into() })
}

/// A plain URL is a tarball when its path has a tarball extension, and a
/// file otherwise.
fn by_extension(location: &str) -> ResourceType {
    if is_tarball(location) {
        ResourceType::Tarball
    } else {
        ResourceType::File
    }
}

/// Maximum path-segment count for the indirect grammar
/// (`id[/ref[/rev]]`), shared by `flake:` URLs and the bare flake-id
/// shorthand.
const INDIRECT_MAX_SEGMENTS: usize = 3;

/// Returns `true` if `s` is a flake id: an ASCII letter followed by ASCII
/// alphanumerics, `-` and `_`.
pub(crate) fn is_flake_id(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Build an indirect kind from its non-empty path segments
/// (`id[/ref[/rev]]`).
///
/// With two segments, the second is a revision when it looks like one and
/// a ref otherwise. With three, the second must be a ref and the third a
/// 40-hex SHA-1 commit.
///
/// `raw_input` is forwarded into [`NixUriError::InvalidUrl`] when the id
/// segment is malformed so the diagnostic carries the original text.
pub(crate) fn indirect_from_segments(
    segments: &[&str],
    raw_input: &str,
) -> NixUriResult<FlakeRefType> {
    if segments.len() > INDIRECT_MAX_SEGMENTS {
        return Err(NixUriError::TooManyIndirectSegments {
            count: segments.len(),
        });
    }
    let id = match segments.first() {
        Some(id) if is_flake_id(id) => (*id).to_string(),
        _ => return Err(NixUriError::InvalidUrl(raw_input.into())),
    };
    let (ref_, rev) = match segments {
        [_] => (None, None),
        [_, value] if looks_like_rev(value) => (None, Some((*value).to_string())),
        [_, value] => (Some(validated_ref_name(value)?), None),
        [_, ref_, rev] => {
            if !is_sha1_hex(rev) {
                return Err(NixUriError::InvalidValue {
                    field: "rev",
                    reason: "expected a 40-hex commit hash in the third indirect segment"
                        .to_string(),
                });
            }
            (Some(validated_ref_name(ref_)?), Some((*rev).to_string()))
        }
        _ => unreachable!("segment count is checked above"),
    };
    Ok(FlakeRefType::Indirect {
        id,
        ref_,
        rev,
        location: RefLocation::PathComponent,
    })
}

/// Parse the part of a git-forge URL after `<platform>:`, query and
/// fragment already split off: `<owner>/<repo>[/<ref-or-rev>]`.
///
/// Empty segments are skipped (`github:/o/r` and
/// `github:o/r/main/` are fine) and every segment after the repo forms the
/// ref (`github:o/r/feature/x` is the ref `feature/x`). Owner and repo are
/// not validated beyond URL syntax. Each segment is percent-decoded after
/// the split, so a gitlab subgroup owner can be written `group%2Fsub`.
fn parse_git_forge(platform: GitForgePlatform, input: &str) -> NixUriResult<GitForge> {
    check_url_path(input)?;
    let segments = input
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| decode_percent(s).map(std::borrow::Cow::into_owned))
        .collect::<NixUriResult<Vec<String>>>()?;
    let (owner, repo, rest) = match segments.as_slice() {
        [owner, repo, rest @ ..] => (owner.clone(), repo.clone(), rest),
        _ => {
            return Err(NixUriError::Unsupported(
                UnsupportedReason::MissingParameter {
                    ty: platform.to_string(),
                    parameter: if segments.is_empty() { "owner" } else { "repo" }.to_string(),
                },
            ));
        }
    };
    let (ref_, rev) = match rest {
        [] => (None, None),
        [value] if looks_like_rev(value) => (None, Some(value.clone())),
        _ => (Some(validated_ref_name(&rest.join("/"))?), None),
    };
    Ok(GitForge {
        platform,
        owner,
        repo,
        ref_,
        rev,
        location: RefLocation::PathComponent,
    })
}

/// Reject a URL path with a character outside RFC 3986's path grammar or
/// a malformed percent-escape.
fn check_url_path(path: &str) -> NixUriResult<()> {
    if !path.chars().all(is_url_path_char) {
        return Err(NixUriError::InvalidUrl(path.into()));
    }
    decode_percent(path).map(|_| ())
}

/// Reject a URL authority whose port is not a number.
///
/// Over ssh that is the SCP-style `user@host:path` location written after
/// `git+ssh://`, which gets its own error.
fn check_authority(location: &str, transport: Option<&TransportLayer>) -> NixUriResult<()> {
    let authority = location.split('/').next().unwrap_or_default();
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    // The port follows the last `:` after any bracketed IPv6 literal.
    let after_brackets = host.rsplit_once(']').map_or(host, |(_, after)| after);
    let Some((_, port)) = after_brackets.split_once(':') else {
        return Ok(());
    };
    if port.chars().all(|c| c.is_ascii_digit()) {
        return Ok(());
    }
    if transport == Some(&TransportLayer::Ssh) {
        return Err(NixUriError::Unsupported(UnsupportedReason::ScpStyleUrl {
            url: location.into(),
        }));
    }
    Err(NixUriError::InvalidUrl(location.into()))
}

impl Display for FlakeRefType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // The ref/rev/ref_location fields on `ResourceUrl` are rendered
            // by `FlakeRef::Display`'s query-segment block (ref/rev only ever
            // serialises as a query parameter for Resource), so they are
            // intentionally not destructured here.
            //
            // `tarball+<transport>://...` and `file+<transport>://...` Display
            // as the bare `<transport>://...` form, matching Nix's behaviour
            // of stripping the application prefix on output. The parser still
            // accepts both spellings on input; auto-classification on parse
            // routes a bare `https://...tar.gz` back into `ResourceType::Tarball`.
            Self::Resource(res) => {
                let strip_res_type =
                    matches!(res.res_type, ResourceType::Tarball | ResourceType::File,)
                        && res.transport_type.is_some();
                if !strip_res_type {
                    write!(f, "{}", res.res_type)?;
                }
                if let Some(transport_type) = &res.transport_type {
                    if strip_res_type {
                        write!(f, "{}", transport_type)?;
                    } else {
                        write!(f, "+{}", transport_type)?;
                    }
                }
                write!(f, "://{}", res.location)
            }
            Self::GitForge(GitForge {
                platform,
                owner,
                repo,
                ref_,
                rev,
                location,
            }) => {
                write!(
                    f,
                    "{platform}:{owner}/{repo}",
                    owner = super::encoding::encode_path_segment(owner),
                    repo = super::encoding::encode_path_segment(repo),
                )?;
                if matches!(location, RefLocation::PathComponent)
                    && let Some(value) = ref_.as_deref().or(rev.as_deref())
                {
                    write!(f, "/{value}")?;
                }
                Ok(())
            }
            Self::Indirect {
                id,
                ref_,
                rev,
                location,
            } => {
                write!(f, "flake:{id}")?;
                if matches!(location, RefLocation::PathComponent) {
                    match (ref_.as_deref(), rev.as_deref()) {
                        (Some(r), Some(v)) => write!(f, "/{r}/{v}")?,
                        (Some(v), None) | (None, Some(v)) => write!(f, "/{v}")?,
                        (None, None) => {}
                    }
                }
                Ok(())
            }
            // `rev` is rendered by `FlakeRef::Display`'s alphabetical
            // query block (Path's rev only has a `?rev=` form), so it is
            // intentionally not destructured here.
            Self::Path { path, .. } => write!(f, "path:{path}"),
        }
    }
}

/// Parse `input` with the public entry point and return its kind.
#[cfg(test)]
fn parse_kind(input: &str) -> NixUriResult<FlakeRefType> {
    crate::FlakeRef::parse(input, crate::Context::FlakeInput).map(|flake_ref| flake_ref.kind)
}

/// Like [`parse_kind`], as `builtins.parseFlakeRef` does. A malformed URL
/// is an error here instead of a relative path.
#[cfg(test)]
fn parse_standalone_kind(input: &str) -> NixUriResult<FlakeRefType> {
    crate::FlakeRef::parse(input, crate::Context::Standalone).map(|flake_ref| flake_ref.kind)
}

#[cfg(test)]
mod inc_parse_vc {
    use crate::TransportLayer;

    use super::*;

    #[test]
    fn parse_git_github_collision() {
        let hub = "github:foo/bar";
        let git = "git:///foo/bar";
        let parsed_hub = parse_kind(hub).unwrap();
        let parsed_git = parse_kind(git).unwrap();
        let expected_hub = FlakeRefType::GitForge(GitForge {
            platform: GitForgePlatform::GitHub,
            owner: "foo".to_string(),
            repo: "bar".to_string(),
            ref_: None,
            rev: None,
            location: RefLocation::PathComponent,
        });
        let expected_git = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Git,
            location: "/foo/bar".to_string(),
            transport_type: None,
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        });

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
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        });
        let expected_filerefpath = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Git,
            location: "/foo/bar".to_string(),
            transport_type: Some(TransportLayer::File),
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        });

        let parsed_refpath = parse_kind(uri).unwrap();
        let file_parsed_refpath = parse_kind(file_uri).unwrap();

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
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        });

        let parsed_refpath = parse_kind(uri).unwrap();

        assert_eq!(expected_refpath, parsed_refpath);
    }

    #[test]
    fn git_https() {
        let uri = "git+https:///foo/bar";
        let expected_refpath = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Git,
            location: "/foo/bar".to_string(),
            transport_type: Some(TransportLayer::Https),
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        });

        let parsed_refpath = parse_kind(uri).unwrap();

        assert_eq!(expected_refpath, parsed_refpath);
    }

    #[test]
    fn hg_file() {
        let file_uri = "hg+file:///foo/bar";
        let file_expected_refpath = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Mercurial,
            location: "/foo/bar".to_string(),
            transport_type: Some(TransportLayer::File),
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        });

        let file_parsed_refpath = parse_kind(file_uri).unwrap();

        assert_eq!(file_expected_refpath, file_parsed_refpath);
    }

    #[test]
    fn hg_http() {
        let uri = "hg+http:///foo/bar";
        let expected_refpath = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Mercurial,
            location: "/foo/bar".to_string(),
            transport_type: Some(TransportLayer::Http),
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        });

        let parsed_refpath = parse_kind(uri).unwrap();

        assert_eq!(expected_refpath, parsed_refpath);
    }

    #[test]
    fn hg_https() {
        let uri = "hg+https:///foo/bar";
        let expected_refpath = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Mercurial,
            location: "/foo/bar".to_string(),
            transport_type: Some(TransportLayer::Https),
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        });

        let parsed_refpath = parse_kind(uri).unwrap();

        assert_eq!(expected_refpath, parsed_refpath);
    }

    #[test]
    fn tarball_https_transport() {
        let uri = "tarball+https://example.com/file.tar.gz";
        let expected = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Tarball,
            location: "example.com/file.tar.gz".to_string(),
            transport_type: Some(TransportLayer::Https),
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        });

        let result = parse_kind(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn tarball_http_transport() {
        let uri = "tarball+http://example.com/file.zip";
        let expected = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Tarball,
            location: "example.com/file.zip".to_string(),
            transport_type: Some(TransportLayer::Http),
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        });

        let result = parse_kind(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn file_https_transport() {
        let uri = "file+https://example.com/file.txt";
        let expected = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::File,
            location: "example.com/file.txt".to_string(),
            transport_type: Some(TransportLayer::Https),
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        });

        let result = parse_kind(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn file_http_transport() {
        let uri = "file+http://example.com/file.txt";
        let expected = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::File,
            location: "example.com/file.txt".to_string(),
            transport_type: Some(TransportLayer::Http),
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        });

        let result = parse_kind(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn bare_git_protocol() {
        let uri = "git://github.com/user/repo.git";
        let expected = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Git,
            location: "github.com/user/repo.git".to_string(),
            transport_type: None,
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        });

        let result = parse_kind(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn plain_https_tarball_autodetect() {
        let uri = "https://example.com/file.tar.gz";
        let expected = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Tarball,
            location: "example.com/file.tar.gz".to_string(),
            transport_type: Some(TransportLayer::Https),
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        });

        let result = parse_kind(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn plain_https_file_autodetect() {
        let uri = "https://example.com/file.txt";
        let expected = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::File,
            location: "example.com/file.txt".to_string(),
            transport_type: Some(TransportLayer::Https),
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        });

        let result = parse_kind(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn plain_http_tarball_autodetect() {
        let uri = "http://example.com/archive.zip";
        let expected = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Tarball,
            location: "example.com/archive.zip".to_string(),
            transport_type: Some(TransportLayer::Http),
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        });

        let result = parse_kind(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn plain_http_file_autodetect() {
        let uri = "http://example.com/README.md";
        let expected = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::File,
            location: "example.com/README.md".to_string(),
            transport_type: Some(TransportLayer::Http),
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        });

        let result = parse_kind(uri).unwrap();
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
            let result = parse_kind(uri).unwrap();
            match result {
                FlakeRefType::Resource(ResourceUrl {
                    res_type: ResourceType::Tarball,
                    ..
                }) => {
                    // Expected.
                }
                _ => panic!("Expected tarball for URI: {}", uri),
            }
        }
    }

    #[test]
    fn case_sensitive_extensions() {
        let uri_lowercase = "https://example.com/file.tar.gz";
        let result_lowercase = parse_kind(uri_lowercase).unwrap();
        match result_lowercase {
            FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::Tarball,
                ..
            }) => {
                // Expected.
            }
            _ => panic!("Expected tarball for lowercase extension"),
        }

        // Uppercase extension should be treated as File, not Tarball.
        let uri_uppercase = "https://example.com/file.TAR.GZ";
        let result_uppercase = parse_kind(uri_uppercase).unwrap();
        match result_uppercase {
            FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::File,
                ..
            }) => {
                // Expected.
            }
            _ => panic!("Expected file for uppercase extension"),
        }
    }
}

#[cfg(test)]
mod inc_parse_flake_id {
    use super::*;

    #[test]
    fn flake_explicit_scheme_simple() {
        let uri = "flake:nixpkgs";
        let expected = FlakeRefType::Indirect {
            id: "nixpkgs".to_string(),
            ref_: None,
            rev: None,
            location: RefLocation::PathComponent,
        };

        let result = parse_kind(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn flake_explicit_scheme_with_ref() {
        let uri = "flake:nixpkgs/release-23.05";
        let expected = FlakeRefType::Indirect {
            id: "nixpkgs".to_string(),
            ref_: Some("release-23.05".to_string()),
            rev: None,
            location: RefLocation::PathComponent,
        };

        let result = parse_kind(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn flake_explicit_scheme_with_hyphens() {
        let uri = "flake:my-flake";
        let expected = FlakeRefType::Indirect {
            id: "my-flake".to_string(),
            ref_: None,
            rev: None,
            location: RefLocation::PathComponent,
        };

        let result = parse_kind(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn flake_explicit_scheme_with_underscores() {
        let uri = "flake:my_flake";
        let expected = FlakeRefType::Indirect {
            id: "my_flake".to_string(),
            ref_: None,
            rev: None,
            location: RefLocation::PathComponent,
        };

        let result = parse_kind(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn flake_explicit_scheme_invalid_start_with_number() {
        let uri = "flake:123invalid";
        let result = parse_standalone_kind(uri);
        assert!(result.is_err());
    }

    #[test]
    fn flake_explicit_scheme_empty() {
        let uri = "flake:";
        let result = parse_standalone_kind(uri);
        assert!(result.is_err());
    }

    #[test]
    fn flake_explicit_scheme_invalid_characters() {
        let uri = "flake:invalid!";
        let result = parse_standalone_kind(uri);
        assert!(result.is_err());
    }

    #[test]
    fn simple_flake_id() {
        let uri = "simple-flake";
        let expected = FlakeRefType::Indirect {
            id: "simple-flake".to_string(),
            ref_: None,
            rev: None,
            location: RefLocation::PathComponent,
        };

        let result = parse_kind(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn flake_id_with_underscores() {
        let uri = "flake_with_underscores";
        let expected = FlakeRefType::Indirect {
            id: "flake_with_underscores".to_string(),
            ref_: None,
            rev: None,
            location: RefLocation::PathComponent,
        };

        let result = parse_kind(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn bare_flake_id_with_numbers() {
        let uri = "nixpkgs23";
        let expected = FlakeRefType::Indirect {
            id: "nixpkgs23".to_string(),
            ref_: None,
            rev: None,
            location: RefLocation::PathComponent,
        };

        let result = parse_kind(uri).unwrap();
        assert_eq!(expected, result);

        // Test via parse() method too.
        let result = parse_kind(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn bare_flake_id_edge_cases() {
        // Test with too many slashes - should fail as indirect, no fallback should work.
        let uri = "my-flake/branch/deep/reference";
        // This should fail because it has too many slashes - only id/ref is allowed for bare IDs.
        let result = parse_kind(uri);
        assert!(
            result.is_err(),
            "Multi-slash URIs should fail when not matching any scheme"
        );

        // Test single character ID.
        let uri = "a";
        let expected = FlakeRefType::Indirect {
            id: "a".to_string(),
            ref_: None,
            rev: None,
            location: RefLocation::PathComponent,
        };
        let result = parse_kind(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn flake_scheme_validation_edge_cases() {
        // Empty ID after flake:.
        let uri = "flake:";
        let result = parse_standalone_kind(uri);
        assert!(result.is_err());

        // ID starting with number.
        let uri = "flake:123invalid";
        let result = parse_standalone_kind(uri);
        assert!(result.is_err());

        // ID with invalid characters.
        let uri = "flake:invalid!";
        let result = parse_standalone_kind(uri);
        assert!(result.is_err());

        // Very long but valid ID.
        let uri = "flake:very-long-flake-name-with-many-dashes-and_underscores_123";
        let expected = FlakeRefType::Indirect {
            id: "very-long-flake-name-with-many-dashes-and_underscores_123".to_string(),
            ref_: None,
            rev: None,
            location: RefLocation::PathComponent,
        };
        let result = parse_standalone_kind(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn protocol_collision_edge_cases() {
        // Ensure git:// doesn't collide with github:.
        let git_uri = "git://example.com/repo.git";
        let github_uri = "github:user/repo";

        let git_result = parse_kind(git_uri).unwrap();
        let github_result = parse_kind(github_uri).unwrap();

        match git_result {
            FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::Git,
                ..
            }) => {
                // Expected.
            }
            _ => panic!("Expected git resource for git:// URL"),
        }

        match github_result {
            FlakeRefType::GitForge(_) => {
                // Expected.
            }
            _ => panic!("Expected git forge for github: URL"),
        }
    }

    #[test]
    fn http_https_autodetection_edge_cases() {
        let test_cases = vec![
            // Valid tarball extensions.
            ("https://example.com/file.tar.gz", ResourceType::Tarball),
            ("https://example.com/file.tar.bz2", ResourceType::Tarball),
            ("https://example.com/file.tar.xz", ResourceType::Tarball),
            ("https://example.com/file.tar.zst", ResourceType::Tarball),
            ("https://example.com/file.tgz", ResourceType::Tarball),
            ("https://example.com/file.zip", ResourceType::Tarball),
            ("https://example.com/file.tar", ResourceType::Tarball),
            // Extensions that are NOT tarball (bare compression formats).
            ("https://example.com/file.gz", ResourceType::File),
            ("https://example.com/file.bz2", ResourceType::File),
            ("https://example.com/file.xz", ResourceType::File),
            // Other file types.
            ("https://example.com/file.txt", ResourceType::File),
            ("https://example.com/README.md", ResourceType::File),
            ("https://example.com/file", ResourceType::File), // No extension.
        ];

        for (uri, expected_type) in test_cases {
            let result = parse_kind(uri).unwrap();
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
        // Test all transport combinations work.
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
            let result = parse_kind(uri).unwrap();
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
            let result = parse_kind(uri).unwrap();
            match result {
                FlakeRefType::Path { path, rev } => {
                    assert_eq!(uri, path, "Path should match input for: {}", uri);
                    assert_eq!(rev, None, "rev should be None for plain path");
                }
                _ => panic!("Expected path for URI: {}", uri),
            }
        }
    }

    #[test]
    fn flake_id_boundary_cases() {
        // Single character flake ID.
        let uri = "a";
        let result = parse_kind(uri).unwrap();
        assert_eq!(
            result,
            FlakeRefType::Indirect {
                id: "a".to_string(),
                ref_: None,
                rev: None,
                location: RefLocation::PathComponent,
            }
        );

        // Flake ID with maximum allowed characters.
        let uri = "abcDEF123-_";
        let result = parse_kind(uri).unwrap();
        assert_eq!(
            result,
            FlakeRefType::Indirect {
                id: "abcDEF123-_".to_string(),
                ref_: None,
                rev: None,
                location: RefLocation::PathComponent,
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
        let result = parse_kind(uri);
        assert!(result.is_err());
    }

    #[test]
    fn error_malformed_url() {
        let uri = "://invalid";
        let result = parse_standalone_kind(uri);
        assert!(result.is_err());
    }

    #[test]
    fn path_with_brackets() {
        // Not a URL, so URL syntax does not apply to a bare path.
        let uri = "/path/with[brackets]";
        assert_eq!(
            parse_standalone_kind(uri).unwrap(),
            FlakeRefType::Path {
                path: uri.to_string(),
                rev: None,
            }
        );
    }

    #[test]
    fn path_with_query_fragment() {
        let uri = "/path/with?query#fragment";
        let result = parse_kind(uri);
        assert!(result.is_err());
    }

    #[test]
    fn file_extension_edge_cases() {
        // File without extension should be treated as File, not Tarball.
        let uri = "https://example.com/README";
        let result = parse_kind(uri).unwrap();
        match result {
            FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::File,
                ..
            }) => {
                // Expected.
            }
            _ => panic!("Expected file resource for extensionless file"),
        }
    }

    #[test]
    fn url_with_port() {
        let uri = "https://example.com:8080/file.tar.gz";
        let result = parse_kind(uri).unwrap();
        match result {
            FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::Tarball,
                location,
                transport_type: Some(TransportLayer::Https),
                ..
            }) => {
                assert_eq!(location, "example.com:8080/file.tar.gz");
            }
            _ => panic!("Expected tarball resource with HTTPS transport"),
        }
    }

    #[test]
    fn mixed_case_domain() {
        let uri = "https://Example.COM/file.tar.gz";
        let result = parse_kind(uri).unwrap();
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
        let result = parse_kind(&uri);

        // Should parse successfully even with very long URLs.
        assert!(result.is_ok());
    }

    #[test]
    fn transport_scheme_combinations() {
        // All valid combinations for tarball.
        let valid_tarballs = vec![
            "tarball+https://example.com/file.tar.gz",
            "tarball+http://example.com/file.tar.gz",
            "tarball+file:///path/to/file.tar.gz",
        ];

        for uri in valid_tarballs {
            let result = parse_kind(uri);
            assert!(result.is_ok(), "Failed to parse valid tarball URI: {}", uri);
        }

        // All valid combinations for file.
        let valid_files = vec![
            "file+https://example.com/file.txt",
            "file+http://example.com/file.txt",
            "file+file:///path/to/file.txt",
        ];

        for uri in valid_files {
            let result = parse_kind(uri);
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
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        });

        let result = parse_kind(uri).unwrap();
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
            rev: None,
        };

        let parsed_refpath = parse_kind(uri).unwrap();

        assert_eq!(expected_refpath, parsed_refpath);
    }

    #[test]
    fn naked_abs() {
        let uri = "/foo/bar";
        let expected_refpath = FlakeRefType::Path {
            path: "/foo/bar".to_string(),
            rev: None,
        };

        let parsed_refpath = parse_kind(uri).unwrap();

        assert_eq!(expected_refpath, parsed_refpath);
    }

    #[test]
    fn relative_path_current_dir() {
        let uri = ".";
        let expected = FlakeRefType::Path {
            path: ".".to_string(),
            rev: None,
        };

        let result = parse_kind(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn relative_path_parent_dir() {
        let uri = "..";
        let expected = FlakeRefType::Path {
            path: "..".to_string(),
            rev: None,
        };

        let result = parse_kind(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn relative_path_current_subdir() {
        let uri = "./relative/path";
        let expected = FlakeRefType::Path {
            path: "./relative/path".to_string(),
            rev: None,
        };

        let result = parse_kind(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn relative_path_parent_subdir() {
        let uri = "../parent/path";
        let expected = FlakeRefType::Path {
            path: "../parent/path".to_string(),
            rev: None,
        };

        let result = parse_kind(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn complex_path_with_dots() {
        let uri = "./path/with/../../complex/structure";
        let expected = FlakeRefType::Path {
            path: "./path/with/../../complex/structure".to_string(),
            rev: None,
        };

        let result = parse_kind(uri).unwrap();
        assert_eq!(expected, result);
    }
}

#[cfg(test)]
mod resource_type_methods {
    use crate::FlakeRef;
    use rstest::rstest;

    #[rstest]
    #[case("git+https://github.com/owner/repo", "github.com", "owner", "repo")]
    #[case(
        "git+https://git.clan.lol/kenji/test-release",
        "git.clan.lol",
        "kenji",
        "test-release"
    )]
    #[case(
        "git+https://codeberg.org/forgejo/forgejo",
        "codeberg.org",
        "forgejo",
        "forgejo"
    )]
    #[case("git+https://gitlab.com/user/project", "gitlab.com", "user", "project")]
    #[case("git+http://example.com/org/myrepo", "example.com", "org", "myrepo")]
    fn test_resource_git_url_extraction(
        #[case] url: &str,
        #[case] expected_domain: &str,
        #[case] expected_owner: &str,
        #[case] expected_repo: &str,
    ) {
        let parsed: FlakeRef = url.parse().unwrap();

        assert_eq!(
            parsed.domain(),
            Some(expected_domain),
            "Domain mismatch for {}",
            url
        );
        assert_eq!(
            parsed.owner(),
            Some(expected_owner),
            "Owner mismatch for {}",
            url
        );
        assert_eq!(
            parsed.repo(),
            Some(expected_repo),
            "Repo mismatch for {}",
            url
        );
        assert_eq!(parsed.id(), Some(expected_repo), "ID mismatch for {}", url);
    }

    #[rstest]
    #[case("git+https://github.com/owner/repo.git", "repo")]
    #[case("git+https://git.clan.lol/kenji/test-release.git", "test-release")]
    fn test_resource_git_url_with_git_suffix(#[case] url: &str, #[case] expected_repo: &str) {
        let parsed: FlakeRef = url.parse().unwrap();

        assert_eq!(
            parsed.repo(),
            Some(expected_repo),
            ".git suffix should be stripped"
        );
        assert_eq!(
            parsed.id(),
            Some(expected_repo),
            ".git suffix should be stripped from ID"
        );
    }

    #[rstest]
    #[case("github:nixos/nixpkgs", "github.com", "nixos", "nixpkgs")]
    #[case("gitlab:owner/repo", "gitlab.com", "owner", "repo")]
    #[case("sourcehut:user/project", "git.sr.ht", "user", "project")]
    fn test_gitforge_domain_extraction(
        #[case] url: &str,
        #[case] expected_domain: &str,
        #[case] expected_owner: &str,
        #[case] expected_repo: &str,
    ) {
        let parsed: FlakeRef = url.parse().unwrap();

        assert_eq!(
            parsed.domain(),
            Some(expected_domain),
            "Domain mismatch for {}",
            url
        );
        assert_eq!(
            parsed.owner(),
            Some(expected_owner),
            "Owner mismatch for {}",
            url
        );
        assert_eq!(
            parsed.repo(),
            Some(expected_repo),
            "Repo mismatch for {}",
            url
        );
    }

    #[rstest]
    #[case("path:/foo/bar")]
    #[case("/foo/bar")]
    #[case("./relative/path")]
    #[case("flake:nixpkgs")]
    #[case("https://example.com/file.tar.gz")]
    fn test_non_git_resource_returns_none(#[case] url: &str) {
        let parsed: FlakeRef = url.parse().unwrap();

        assert_eq!(
            parsed.domain(),
            None,
            "Non-git resources should return None for domain"
        );
        assert_eq!(
            parsed.owner(),
            None,
            "Non-git resources should return None for owner"
        );
        assert_eq!(
            parsed.repo(),
            None,
            "Non-git resources should return None for repo"
        );
    }

    #[rstest]
    #[case(
        "git+https://example.com/a/b",
        Some("example.com"),
        Some("a"),
        Some("b")
    )]
    #[case("git+https://x.y.z/org/repo", Some("x.y.z"), Some("org"), Some("repo"))]
    #[case("git+https://host/o/r.git", Some("host"), Some("o"), Some("r"))]
    fn test_resource_url_minimal_parsing(
        #[case] url: &str,
        #[case] expected_domain: Option<&str>,
        #[case] expected_owner: Option<&str>,
        #[case] expected_repo: Option<&str>,
    ) {
        let parsed: FlakeRef = url.parse().unwrap();
        assert_eq!(parsed.domain(), expected_domain);
        assert_eq!(parsed.owner(), expected_owner);
        assert_eq!(parsed.repo(), expected_repo);
    }

    #[rstest]
    #[case("git+https://domain.com/owner")] // Missing repo.
    #[case("git+https://domain.com")] // Missing owner and repo.
    fn test_resource_url_insufficient_components_returns_none(#[case] url: &str) {
        let parsed: FlakeRef = url.parse().unwrap();
        // With insufficient path components, should return None.
        assert!(
            parsed.repo().is_none() || parsed.owner().is_none(),
            "URLs with insufficient path components should return None for missing parts"
        );
    }

    #[rstest]
    #[case::git_https_default_port_returns_host_only("git+https://example.com/o/r", "example.com")]
    #[case::git_https_non_default_port_returns_host_with_port(
        "git+https://localhost:3000/o/r",
        "localhost:3000"
    )]
    #[case::git_https_explicit_default_port_strips(
        "git+https://example.com:443/o/r",
        "example.com"
    )]
    #[case::git_ssh_default_port_returns_host_only("git+ssh://example.com/o/r", "example.com")]
    #[case::git_ssh_non_default_port_returns_host_with_port(
        "git+ssh://example.com:2222/o/r",
        "example.com:2222"
    )]
    #[case::git_ssh_explicit_default_port_strips("git+ssh://example.com:22/o/r", "example.com")]
    #[case::git_http_non_default_port_returns_host_with_port(
        "git+http://example.com:8080/o/r",
        "example.com:8080"
    )]
    fn domain_retains_non_default_port(#[case] url: &str, #[case] expected: &str) {
        // Mirrors upstream HTTP-library `Authority` semantics: the public
        // `domain()` accessor reflects what a downstream fetcher must
        // target, so a non-default `:port` survives. `flake-edit` consumes
        // `domain()` directly as `api_host_for(domain)` input; without
        // this, self-hosted forges on non-default ports silently fetched
        // from the scheme's default port.
        let parsed: FlakeRef = url.parse().unwrap();
        assert_eq!(parsed.domain(), Some(expected), "domain mismatch for {url}");
    }

    #[rstest]
    #[case("git+ssh://git@host/owner/repo", "host")]
    #[case("git+https://host/owner/repo", "host")]
    fn domain_strips_user_and_path(#[case] url: &str, #[case] expected_host: &str) {
        // Pre-fix `domain` did `location.split('/').next()`, which for an
        // SCP-style SSH location like `git@host:owner/repo` returned
        // `git@host:owner` rather than `host`. The fix strips any leading
        // `user@` and stops at the first `:` or `/`.
        let parsed: FlakeRef = url.parse().unwrap();
        assert_eq!(
            parsed.domain(),
            Some(expected_host),
            "domain mismatch for {url}"
        );
    }
}
