use std::{fmt::Display, path::Path};

use serde::{Deserialize, Serialize};
use winnow::{
    ModalResult, Parser,
    combinator::{alt, opt, peek, preceded, separated_pair, terminated},
    error::{StrContext, StrContextValue},
    token::{rest, take_till, take_until},
};

use crate::{
    error::{NixUriError, NixUriResult, UnsupportedReason, run_partial, tag},
    flakeref::{
        RefLocation, TransportLayer,
        encoding::decode_percent,
        forge::{GitForge, validate_owner_repo},
        validators::{looks_like_rev, validated_ref_name},
    },
    parser::parse_transport_type,
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
    #[allow(dead_code)]
    pub(crate) fn parse_path(input: &mut &str) -> ModalResult<Self> {
        preceded(
            opt(alt((tag("path://"), tag("path:")))),
            Self::path_parser.map(|path_str| Self::Path {
                path: path_str.to_string(),
                rev: None,
            }),
        )
        .parse_next(input)
    }

    // TODO: #158
    #[allow(dead_code)]
    pub(crate) fn parse_file(input: &mut &str) -> ModalResult<Self> {
        alt((
            // Handle file+http[s]:// as Resource with proper transport.
            Self::parse_file_with_http_transport,
            Self::parse_explicit_file_scheme.map(|path| {
                Self::Resource(ResourceUrl::new(
                    ResourceType::File,
                    path.display().to_string(),
                    None,
                ))
            }),
            Self::parse_naked.map(|path| Self::Path {
                path: format!("{}", path.display()),
                rev: None,
            }),
        ))
        .context(StrContext::Label("path resource"))
        .parse_next(input)
    }

    #[allow(dead_code)]
    pub(crate) fn parse_naked<'i>(input: &mut &'i str) -> ModalResult<&'i Path> {
        // Check that input starts with `.` or `/`.
        peek(alt(('.', '/')))
            .context(StrContext::Label("path location"))
            .parse_next(input)?;
        let path_str = Self::path_parser.parse_next(input)?;
        Ok(Path::new(path_str))
    }

    #[allow(dead_code)]
    pub(crate) fn path_parser<'i>(input: &mut &'i str) -> ModalResult<&'i str> {
        preceded(peek(alt(('.', '/'))), Self::path_verifier).parse_next(input)
    }

    #[allow(dead_code)]
    pub(crate) fn path_verifier<'i>(input: &mut &'i str) -> ModalResult<&'i str> {
        take_till(0.., |c| c == '#' || c == '?')
            .verify(|c: &&str| !c.contains('[') && !c.contains(']'))
            .context(StrContext::Label("path validation"))
            .parse_next(input)
    }

    #[allow(dead_code)]
    pub(crate) fn parse_explicit_file_scheme<'i>(input: &mut &'i str) -> ModalResult<&'i Path> {
        preceded(
            tag("file"),
            preceded(
                opt(tag("+file")),
                terminated(
                    ':'.context(StrContext::Expected(StrContextValue::CharLiteral(':'))),
                    opt(tag("//")),
                ),
            ),
        )
        .context(StrContext::Label("file resource"))
        .parse_next(input)?;
        let path_str = Self::path_parser.parse_next(input)?;
        Ok(Path::new(path_str))
    }

    #[allow(dead_code)]
    pub(crate) fn parse_file_with_http_transport(input: &mut &str) -> ModalResult<Self> {
        let scheme = alt((tag("file+https"), tag("file+http"))).parse_next(input)?;
        let _ = tag("://").parse_next(input)?;
        let location = take_till(0.., |c| c == '#' || c == '?').parse_next(input)?;

        let transport_type = match scheme {
            "file+https" => Some(TransportLayer::Https),
            "file+http" => Some(TransportLayer::Http),
            _ => unreachable!(),
        };

        Ok(Self::Resource(ResourceUrl::new(
            ResourceType::File,
            location.to_string(),
            transport_type,
        )))
    }

    /// TODO: different platforms have different rules about owner/repo/ref
    /// strings; not enforced today.
    /// `<github | gitlab | sourcehut>:<owner>/<repo>[/<rev | ref>]...`
    #[allow(dead_code)]
    pub(crate) fn parse_git_forge(input: &mut &str) -> ModalResult<Self> {
        GitForge::parse.map(Self::GitForge).parse_next(input)
    }

    /// `<git | hg>[+<transport-type>]://...`
    #[allow(dead_code)]
    pub(crate) fn parse_resource(input: &mut &str) -> ModalResult<Self> {
        ResourceUrl::parse.map(Self::Resource).parse_next(input)
    }

    /// Parse plain HTTP/HTTPS URLs with auto-detection.
    #[allow(dead_code)]
    pub(crate) fn parse_plain_url(input: &mut &str) -> ModalResult<Self> {
        use crate::parser::is_tarball;

        let scheme = alt((tag("https"), tag("http"))).parse_next(input)?;
        let _ = tag("://").parse_next(input)?;
        let location = take_till(0.., |c| c == '#' || c == '?')
            .context(StrContext::Label("url location"))
            .parse_next(input)?;

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

        Ok(Self::Resource(ResourceUrl::new(
            res_type,
            location.to_string(),
            transport_type,
        )))
    }

    /// Parse type-specific information; the production entry point for
    /// classifying a URI's kind.
    #[allow(dead_code)]
    pub(crate) fn parse_type(input: &str) -> NixUriResult<Self> {
        let (_, maybe_explicit_type) = run_partial(
            input,
            input,
            opt(separated_pair(take_until(0.., ":"), ':', rest)),
        )?;
        if let Some((flake_ref_type_str, rest_input)) = maybe_explicit_type {
            match flake_ref_type_str {
                "github" | "gitlab" | "sourcehut" => {
                    let (_input, owner_and_repo_or_ref) =
                        run_partial(input, rest_input, GitForge::parse_owner_repo_ref)?;
                    // Match Nix's per-segment percent-decode: split on raw
                    // `/` boundaries, then percent-decode each segment.
                    // Decoding *after* the split is what lets a `%2F` inside an
                    // owner survive as a literal `/` without colliding with the
                    // owner-vs-repo separator. The validator then accepts that
                    // `/` only when the platform is `gitlab:` (subgroup form);
                    // see [`validate_owner_repo`].
                    let owner = decode_percent(owner_and_repo_or_ref.0)?.into_owned();
                    let repo = decode_percent(owner_and_repo_or_ref.1)?.into_owned();
                    let (ref_, rev) = match owner_and_repo_or_ref.2 {
                        Some(v) => {
                            let v = decode_percent(v)?.into_owned();
                            if looks_like_rev(&v) {
                                (None, Some(v))
                            } else {
                                (Some(validated_ref_name(&v)?), None)
                            }
                        }
                        None => (None, None),
                    };
                    let platform = match flake_ref_type_str {
                        "github" => GitForgePlatform::GitHub,
                        "gitlab" => GitForgePlatform::GitLab,
                        "sourcehut" => GitForgePlatform::SourceHut,
                        _ => unreachable!(),
                    };
                    validate_owner_repo(&platform, &owner, &repo)?;
                    let res = Self::GitForge(GitForge {
                        platform,
                        owner,
                        repo,
                        ref_,
                        rev,
                        location: RefLocation::PathComponent,
                    });
                    Ok(res)
                }
                "path" => {
                    // Nix rejects `path://` only when the URL authority's
                    // host is non-empty. An *empty* authority (the body
                    // begins `///` or is just `//`) decodes to the
                    // trailing path and is accepted; e.g. `path:///abs`
                    // parses as the absolute path `/abs`. So inspect
                    // what follows the second `/` rather than rejecting
                    // any `//` prefix outright. The body is stored
                    // verbatim (leading slashes preserved) so Display
                    // round-trips; the slash-collapse normalisation
                    // Nix performs is intentionally deferred.
                    if let Some(after) = rest_input.strip_prefix("//") {
                        let host_end = after.find('/').unwrap_or(after.len());
                        if !after[..host_end].is_empty() {
                            return Err(NixUriError::Unsupported(UnsupportedReason::Authority {
                                scheme: "path",
                            }));
                        }
                    }
                    if rest_input.contains(']') || rest_input.contains('[') {
                        return Err(NixUriError::InvalidUrl(rest_input.into()));
                    }
                    if rest_input.is_empty() || rest_input.trim().is_empty() {
                        return Err(NixUriError::InvalidUrl(rest_input.into()));
                    }
                    if rest_input.contains('#') || rest_input.contains('?') {
                        return Err(NixUriError::InvalidUrl(rest_input.into()));
                    }
                    let flake_ref_type = Self::Path {
                        path: rest_input.into(),
                        rev: None,
                    };
                    Ok(flake_ref_type)
                }
                "flake" => {
                    // Nix skips empty segments when splitting the URL
                    // path, so `flake:nixpkgs//main` collapses to
                    // `flake:nixpkgs/main`. Match that here.
                    let segments: Vec<&str> =
                        rest_input.split('/').filter(|s| !s.is_empty()).collect();
                    if segments.is_empty() {
                        return Err(NixUriError::InvalidUrl(rest_input.into()));
                    }
                    if segments.len() > INDIRECT_MAX_SEGMENTS {
                        return Err(NixUriError::TooManyIndirectSegments {
                            count: segments.len(),
                        });
                    }
                    let (id, ref_, rev) = classify_indirect_segments(&segments, rest_input)?;
                    Ok(Self::Indirect {
                        id,
                        ref_,
                        rev,
                        location: RefLocation::PathComponent,
                    })
                }

                _ => {
                    if flake_ref_type_str.starts_with("git+") {
                        let transport_type = parse_transport_type(flake_ref_type_str)?;
                        let (rest_input, _tag) = run_partial(input, rest_input, opt(tag("//")))?;
                        let flake_ref_type = Self::Resource(ResourceUrl::new(
                            ResourceType::Git,
                            rest_input.into(),
                            Some(transport_type),
                        ));
                        Ok(flake_ref_type)
                    } else if flake_ref_type_str.starts_with("hg+") {
                        let transport_type = parse_transport_type(flake_ref_type_str)?;
                        let (rest_input, _tag) = run_partial(input, rest_input, tag("//"))?;
                        let flake_ref_type = Self::Resource(ResourceUrl::new(
                            ResourceType::Mercurial,
                            rest_input.into(),
                            Some(transport_type),
                        ));
                        Ok(flake_ref_type)
                    } else if flake_ref_type_str.starts_with("tarball+") {
                        let transport_type = parse_transport_type(flake_ref_type_str)?;
                        let (rest_input, _tag) = run_partial(input, rest_input, opt(tag("//")))?;
                        let flake_ref_type = Self::Resource(ResourceUrl::new(
                            ResourceType::Tarball,
                            rest_input.into(),
                            Some(transport_type),
                        ));
                        Ok(flake_ref_type)
                    } else if flake_ref_type_str.starts_with("file+") {
                        let transport_type = parse_transport_type(flake_ref_type_str)?;
                        let (rest_input, _tag) = run_partial(input, rest_input, opt(tag("//")))?;
                        let flake_ref_type = Self::Resource(ResourceUrl::new(
                            ResourceType::File,
                            rest_input.into(),
                            Some(transport_type),
                        ));
                        Ok(flake_ref_type)
                    } else if flake_ref_type_str == "https" || flake_ref_type_str == "http" {
                        // Plain HTTP/HTTPS URL - auto-detect type based on extension.
                        use crate::parser::is_tarball;

                        let (rest_input, _tag) = run_partial(input, rest_input, tag("//"))?;
                        let res_type = if is_tarball(rest_input) {
                            ResourceType::Tarball
                        } else {
                            ResourceType::File
                        };
                        let transport_type = match flake_ref_type_str {
                            "https" => Some(TransportLayer::Https),
                            "http" => Some(TransportLayer::Http),
                            _ => None,
                        };

                        let flake_ref_type = Self::Resource(ResourceUrl::new(
                            res_type,
                            rest_input.into(),
                            transport_type,
                        ));
                        Ok(flake_ref_type)
                    } else if flake_ref_type_str == "file" {
                        // Bare `file://` URL: Nix routes plain `file:`
                        // through the same tarball-extension classifier
                        // as `http(s)://`, splitting between the file
                        // (no extension) and tarball (extension present)
                        // shapes. The transport is always `file`, so a
                        // round-trip from the explicit `tarball+file://`
                        // shape (whose Display strips `tarball+`)
                        // lands back on the same kind.
                        use crate::parser::is_tarball;

                        let (rest_input, _tag) = run_partial(input, rest_input, tag("//"))?;
                        let res_type = if is_tarball(rest_input) {
                            ResourceType::Tarball
                        } else {
                            ResourceType::File
                        };
                        let flake_ref_type = Self::Resource(ResourceUrl::new(
                            res_type,
                            rest_input.into(),
                            Some(TransportLayer::File),
                        ));
                        Ok(flake_ref_type)
                    } else if flake_ref_type_str == "git" {
                        // Bare git:// protocol: native git over the wire,
                        // no `+<transport>` layer.
                        let (rest_input, _tag) = run_partial(input, rest_input, tag("//"))?;
                        let flake_ref_type = Self::Resource(ResourceUrl::new(
                            ResourceType::Git,
                            rest_input.into(),
                            None,
                        ));
                        Ok(flake_ref_type)
                    } else {
                        Err(NixUriError::Unsupported(UnsupportedReason::UriType {
                            ty: flake_ref_type_str.into(),
                        }))
                    }
                }
            }
        } else {
            // Implicit types can be paths or indirect flake-ids. The bare
            // form matches Nix's bare-flake-id shape, which routes
            // matching shapes through the same indirect scheme as
            // `flake:`.
            if input.starts_with('/')
                || input.starts_with("./")
                || input.starts_with("../")
                || input == "."
                || input == ".."
            {
                // Bare `//host/path` has no Nix grammar (Nix only
                // recognises single-slash absolute and relative
                // shapes for bare path flake-refs). Without this
                // guard the body was stored verbatim and Display
                // emitted `path://host/path`, which the `path:` arm
                // then rejects on re-parse via the authority guard.
                if input.starts_with("//") {
                    return Err(NixUriError::InvalidUrl(input.into()));
                }
                let flake_ref_type = Self::Path {
                    path: input.into(),
                    rev: None,
                };
                if input.contains(']')
                    || input.contains('[')
                    || !input.is_ascii()
                    || input.contains('#')
                    || input.contains('?')
                {
                    return Err(NixUriError::InvalidUrl(input.into()));
                }
                return Ok(flake_ref_type);
            }

            // Bare indirect form. Nix's bare-flake-id regex does not
            // permit empty path segments (the `flake:` URL form skips
            // empties, but the bare form is matched by a regex that
            // requires non-empty content), so reject any.
            let segments: Vec<&str> = input.split('/').collect();
            if segments.iter().any(|s| s.is_empty()) {
                return Err(NixUriError::InvalidUrl(input.into()));
            }
            if segments.len() > INDIRECT_MAX_SEGMENTS {
                return Err(NixUriError::MissingScheme {
                    input: input.into(),
                });
            }
            let (id, ref_, rev) = classify_indirect_segments(&segments, input)?;
            Ok(Self::Indirect {
                id,
                ref_,
                rev,
                location: RefLocation::PathComponent,
            })
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
                // URL form is `[user@]host[:port]/owner/repo`; the
                // SCP-like SSH form (handled by `parse_scp_style` at
                // entry, but a tolerant path also accepts the inline
                // `host:owner/repo` shape) reuses `:` as a path
                // separator, not a port. Strip any leading `user@`,
                // take everything before the first `/`, then split on
                // the first `:` and discriminate: a numeric segment is
                // a port (kept verbatim when non-default for the
                // scheme, dropped when it matches the default), a
                // non-numeric segment is the SCP-style path separator
                // (drop everything from `:` onwards).
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
                let is_numeric_port =
                    !port_str.is_empty() && port_str.bytes().all(|b| b.is_ascii_digit());
                if !is_numeric_port {
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

/// Maximum path-segment count for the indirect grammar
/// (`id[/ref[/rev]]`). Matches Nix's three-segment cap; the bare
/// (no-scheme) form uses the same cap because Nix's bare-flake-id form
/// routes through the same indirect scheme.
const INDIRECT_MAX_SEGMENTS: usize = 3;

/// Validate a non-empty, non-overflowing slice of indirect path segments
/// and project them onto `(id, ref_, rev)` per Nix's indirect scheme
/// rules.
///
/// Caller is responsible for the segment-count cap and for filtering
/// empty segments where appropriate (the `flake:` URL form filters
/// empties; the bare form does not, since Nix's bare-flake-id regex does
/// not permit empties). Caller also picks the right "too many" error:
/// `TooManyIndirectSegments` from the `flake:` arm, `MissingScheme` from
/// the bare arm.
///
/// `raw_input` is forwarded into [`NixUriError::InvalidUrl`] when the id
/// segment is malformed so the diagnostic carries the original surface
/// text.
fn classify_indirect_segments(
    segments: &[&str],
    raw_input: &str,
) -> Result<(String, Option<String>, Option<String>), NixUriError> {
    let id = segments
        .first()
        .copied()
        .ok_or_else(|| NixUriError::InvalidUrl(raw_input.into()))?;
    if id.is_empty()
        || !id.chars().next().unwrap().is_ascii_alphabetic()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(NixUriError::InvalidUrl(raw_input.into()));
    }

    match segments.len() {
        1 => Ok((id.to_string(), None, None)),
        2 => {
            let v = segments[1];
            if looks_like_rev(v) {
                Ok((id.to_string(), None, Some(v.to_string())))
            } else {
                Ok((id.to_string(), Some(validated_ref_name(v)?), None))
            }
        }
        3 => {
            let r = validated_ref_name(segments[1])?;
            // Nix requires the third segment to be a commit hash;
            // without this check a non-hex value was silently folded
            // back into the ref name (the `rsplit_once` path would yield
            // e.g. `ref_=Some("release-23.05/notahex")`).
            // `looks_like_rev` accepts 40-hex (SHA-1) or 64-hex
            // (SHA-256).
            if !looks_like_rev(segments[2]) {
                return Err(NixUriError::InvalidValue {
                    field: "rev",
                    reason: "expected 40-hex (SHA-1) or 64-hex (SHA-256) commit in third indirect segment".to_string(),
                });
            }
            Ok((id.to_string(), Some(r), Some(segments[2].to_string())))
        }
        _ => unreachable!("caller must enforce segment count <= INDIRECT_MAX_SEGMENTS"),
    }
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
                let owner_out = super::encoding::encode_path_segment(owner);
                write!(f, "{platform}:{owner_out}/{repo}")?;
                if matches!(location, RefLocation::PathComponent) {
                    if let Some(value) = ref_.as_deref().or(rev.as_deref()) {
                        write!(f, "/{value}")?;
                    }
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

#[cfg(test)]
mod inc_parse_vc {
    use crate::TransportLayer;

    use super::*;

    #[test]
    fn parse_git_github_collision() {
        let hub = "github:foo/bar";
        let git = "git:///foo/bar";
        let parsed_hub = FlakeRefType::parse_type(hub).unwrap();
        let parsed_git = FlakeRefType::parse_type(git).unwrap();
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

        let parsed_refpath = FlakeRefType::parse_type(uri).unwrap();
        let file_parsed_refpath = FlakeRefType::parse_type(file_uri).unwrap();

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

        let parsed_refpath = FlakeRefType::parse_type(uri).unwrap();

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

        let parsed_refpath = FlakeRefType::parse_type(uri).unwrap();

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

        let file_parsed_refpath = FlakeRefType::parse_type(file_uri).unwrap();

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

        let parsed_refpath = FlakeRefType::parse_type(uri).unwrap();

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

        let parsed_refpath = FlakeRefType::parse_type(uri).unwrap();

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
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
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
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
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
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
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
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
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
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
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
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
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
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
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
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
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
                    // Expected.
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
                // Expected.
            }
            _ => panic!("Expected tarball for lowercase extension"),
        }

        // Uppercase extension should be treated as File, not Tarball.
        let uri_uppercase = "https://example.com/file.TAR.GZ";
        let result_uppercase = FlakeRefType::parse_type(uri_uppercase).unwrap();
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

        let result = FlakeRefType::parse_type(uri).unwrap();
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

        let result = FlakeRefType::parse_type(uri).unwrap();
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

        let result = FlakeRefType::parse_type(uri).unwrap();
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
            ref_: None,
            rev: None,
            location: RefLocation::PathComponent,
        };

        let result = FlakeRefType::parse_type(uri).unwrap();
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

        let result = FlakeRefType::parse_type(uri).unwrap();
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

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);

        // Test via parse() method too.
        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn bare_flake_id_edge_cases() {
        // Test with too many slashes - should fail as indirect, no fallback should work.
        let uri = "my-flake/branch/deep/reference";
        // This should fail because it has too many slashes - only id/ref is allowed for bare IDs.
        let result = FlakeRefType::parse_type(uri);
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
        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn flake_scheme_validation_edge_cases() {
        // Empty ID after flake:.
        let uri = "flake:";
        let result = FlakeRefType::parse_type(uri);
        assert!(result.is_err());

        // ID starting with number.
        let uri = "flake:123invalid";
        let result = FlakeRefType::parse_type(uri);
        assert!(result.is_err());

        // ID with invalid characters.
        let uri = "flake:invalid!";
        let result = FlakeRefType::parse_type(uri);
        assert!(result.is_err());

        // Very long but valid ID.
        let uri = "flake:very-long-flake-name-with-many-dashes-and_underscores_123";
        let expected = FlakeRefType::Indirect {
            id: "very-long-flake-name-with-many-dashes-and_underscores_123".to_string(),
            ref_: None,
            rev: None,
            location: RefLocation::PathComponent,
        };
        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn protocol_collision_edge_cases() {
        // Ensure git:// doesn't collide with github:.
        let git_uri = "git://example.com/repo.git";
        let github_uri = "github:user/repo";

        let git_result = FlakeRefType::parse_type(git_uri).unwrap();
        let github_result = FlakeRefType::parse_type(github_uri).unwrap();

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
            let result = FlakeRefType::parse_type(uri).unwrap();
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
            let result = FlakeRefType::parse_type(uri).unwrap();
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
            let result = FlakeRefType::parse_type(uri).unwrap();
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
        let result = FlakeRefType::parse_type(uri).unwrap();
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
        let result = FlakeRefType::parse_type(uri).unwrap();
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
        // File without extension should be treated as File, not Tarball.
        let uri = "https://example.com/README";
        let result = FlakeRefType::parse_type(uri).unwrap();
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
        let result = FlakeRefType::parse_type(uri).unwrap();
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
            let result = FlakeRefType::parse_type(uri);
            assert!(result.is_ok(), "Failed to parse valid tarball URI: {}", uri);
        }

        // All valid combinations for file.
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
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
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
            rev: None,
        };

        let parsed_refpath = FlakeRefType::parse_type(uri).unwrap();

        assert_eq!(expected_refpath, parsed_refpath);
    }

    #[test]
    fn naked_abs() {
        let uri = "/foo/bar";
        let expected_refpath = FlakeRefType::Path {
            path: "/foo/bar".to_string(),
            rev: None,
        };

        let parsed_refpath = FlakeRefType::parse_type(uri).unwrap();

        assert_eq!(expected_refpath, parsed_refpath);
    }

    #[test]
    fn relative_path_current_dir() {
        let uri = ".";
        let expected = FlakeRefType::Path {
            path: ".".to_string(),
            rev: None,
        };

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn relative_path_parent_dir() {
        let uri = "..";
        let expected = FlakeRefType::Path {
            path: "..".to_string(),
            rev: None,
        };

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn relative_path_current_subdir() {
        let uri = "./relative/path";
        let expected = FlakeRefType::Path {
            path: "./relative/path".to_string(),
            rev: None,
        };

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn relative_path_parent_subdir() {
        let uri = "../parent/path";
        let expected = FlakeRefType::Path {
            path: "../parent/path".to_string(),
            rev: None,
        };

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn complex_path_with_dots() {
        let uri = "./path/with/../../complex/structure";
        let expected = FlakeRefType::Path {
            path: "./path/with/../../complex/structure".to_string(),
            rev: None,
        };

        let result = FlakeRefType::parse_type(uri).unwrap();
        assert_eq!(expected, result);
    }

    #[test]
    fn naked_cwd() {
        let uri = "./foo/bar";
        let expected_refpath = FlakeRefType::Path {
            path: "./foo/bar".to_string(),
            rev: None,
        };

        let (_rest, parsed_refpath) = FlakeRefType::parse_file.parse_peek(uri).unwrap();

        assert_eq!(expected_refpath, parsed_refpath);
    }

    #[test]
    fn http_layer() {
        let uri = "file+http://example.com/file.txt";
        let expected_refpath = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::File,
            location: "example.com/file.txt".to_string(),
            transport_type: Some(TransportLayer::Http),
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        });

        let (_rest, parsed_refpath) = FlakeRefType::parse_file.parse_peek(uri).unwrap();

        assert_eq!(expected_refpath, parsed_refpath);
    }

    #[test]
    fn https_layer() {
        let uri = "file+https://example.com/file.txt";
        let expected_refpath = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::File,
            location: "example.com/file.txt".to_string(),
            transport_type: Some(TransportLayer::Https),
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        });

        let (_rest, parsed_refpath) = FlakeRefType::parse_file.parse_peek(uri).unwrap();

        assert_eq!(expected_refpath, parsed_refpath);
    }

    #[test]
    fn file_layer() {
        let uri = "file+file:///foo/bar";
        let expected_refpath = FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::File,
            location: "/foo/bar".to_string(),
            transport_type: None,
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        });

        let (_rest, parsed_refpath) = FlakeRefType::parse_file.parse_peek(uri).unwrap();

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
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        };

        let (_rest, parsed_ref) = FlakeRefType::parse_file.parse_peek(path_uri).unwrap();
        let (_rest, parsed_ref2) = FlakeRefType::parse_file.parse_peek(path_uri2).unwrap();

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
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        };

        let (rest, parsed_file) = FlakeRefType::parse_file.parse_peek(path_uri).unwrap();
        assert_eq!(rest, "?");
        let (rest, parsed_file2) = FlakeRefType::parse_file.parse_peek(path_uri2).unwrap();

        assert_eq!(rest, "?");
        assert_eq!(FlakeRefType::Resource(expected_ref.clone()), parsed_file);
        expected_ref.location = "/wheres/wally/".to_string();
        assert_eq!(FlakeRefType::Resource(expected_ref), parsed_file2);
    }

    #[test]
    fn param_term() {
        let path_uri = "file:///wheres/wally?foo=bar#fizz";
        let path_uri2 = "file:///wheres/wally/?foo=bar#fizz";

        let (rest, parsed_file) = FlakeRefType::parse_file.parse_peek(path_uri).unwrap();
        assert_eq!(rest, "?foo=bar#fizz");
        let (rest, parsed_file2) = FlakeRefType::parse_file.parse_peek(path_uri2).unwrap();
        assert_eq!(rest, "?foo=bar#fizz");

        let mut expected_ref = ResourceUrl {
            res_type: ResourceType::File,
            location: "/wheres/wally".to_string(),
            transport_type: None,
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        };
        assert_eq!(FlakeRefType::Resource(expected_ref.clone()), parsed_file);
        expected_ref.location = "/wheres/wally/".to_string();
        assert_eq!(FlakeRefType::Resource(expected_ref), parsed_file2);
    }

    #[test]
    fn empty_param_attr_term() {
        let path_uri = "file:///wheres/wally?#";
        let path_uri2 = "file:///wheres/wally/?#";

        let (rest, parsed_file) = FlakeRefType::parse_file.parse_peek(path_uri).unwrap();
        assert_eq!(rest, "?#");
        let (rest, parsed_file2) = FlakeRefType::parse_file.parse_peek(path_uri2).unwrap();
        assert_eq!(rest, "?#");

        let mut expected_ref = ResourceUrl {
            res_type: ResourceType::File,
            location: "/wheres/wally".to_string(),
            transport_type: None,
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        };
        assert_eq!(FlakeRefType::Resource(expected_ref.clone()), parsed_file);
        expected_ref.location = "/wheres/wally/".to_string();
        assert_eq!(FlakeRefType::Resource(expected_ref.clone()), parsed_file2);

        let path_uri = "file:///wheres/wally#?";
        let path_uri2 = "file:///wheres/wally/#?";

        let (rest, parsed_file) = FlakeRefType::parse_file.parse_peek(path_uri).unwrap();
        assert_eq!(rest, "#?");
        let (rest, parsed_file2) = FlakeRefType::parse_file.parse_peek(path_uri2).unwrap();
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

        let (rest, parsed_file) = FlakeRefType::parse_file.parse_peek(path_uri).unwrap();
        assert_eq!(rest, "#");
        let (rest, parsed_file2) = FlakeRefType::parse_file.parse_peek(path_uri2).unwrap();
        assert_eq!(rest, "#");

        let mut expected_ref = ResourceUrl {
            res_type: ResourceType::File,
            location: "/wheres/wally".to_string(),
            transport_type: None,
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        };
        assert_eq!(FlakeRefType::Resource(expected_ref.clone()), parsed_file);
        expected_ref.location = "/wheres/wally/".to_string();
        assert_eq!(FlakeRefType::Resource(expected_ref), parsed_file2);
        assert_eq!(rest, "#");
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
    #[case("git+ssh://git@host:owner/repo", "host")]
    #[case("git+ssh://host:owner/repo", "host")]
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
