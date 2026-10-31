use std::borrow::Cow;

use crate::{
    Context,
    error::{NixUriError, NixUriResult, UnsupportedReason},
    flakeref::{
        DOWNLOAD_TRANSPORTS, Download, FlakeRef, FlakeRefType, GitForge, GitForgePlatform, GitRepo,
        HG_TRANSPORTS, Indirect, MercurialRepo, PathRef, encoding,
        hash::is_sha1_hash,
        is_flake_id, nul_in_path,
        url::{Authority, Url, parse_query, scheme_of, split_scheme},
        validators::{is_sha1_hex, matches_ref_shape, validated_host_name, validated_ref_name},
    },
};

/// Parse a flake reference in `context`, and return it with its percent-decoded fragment,
/// `None` when there is none or it is empty.
///
/// Three readings are tried in order:
///
/// 1. the flake-id shorthand `id[/ref-or-rev]` (`nixpkgs/nixos-25.05`),
/// 2. a URL with a scheme (`github:`, `git+https:`, `path:`, ...),
/// 3. a path, with an optional query.
///
/// A URL that is malformed falls through to the path reading, which in a
/// [`Context::FlakeInput`] keeps it as a relative path (`github:NixOS/nixpkgs/a..b` is
/// the directory of that name), whatever its scheme. Other URL failures, such as a
/// well-formed URL with an unknown scheme, are final.
pub(crate) fn parse_nix_uri(
    input: &str,
    context: Context,
) -> NixUriResult<(FlakeRef, Option<String>)> {
    let (head, fragment) = match input.split_once('#') {
        Some((head, fragment)) => (head, Some(encoding::decode_percent(fragment)?.into_owned())),
        None => (input, None),
    };

    let flake_ref = match parse_flake_id(head) {
        Some(parsed) => parsed?,
        None => match parse_url(head) {
            Ok(parsed) => parsed,
            Err(UrlFailure::Final(err)) => return Err(err),
            // When the path reading fails too, the URL error says more
            // about what the input was meant to be.
            Err(UrlFailure::Malformed(url_err)) => {
                parse_path(head, context).map_err(|path_err| url_err.unwrap_or(path_err))?
            }
        },
    };
    Ok((flake_ref, fragment.filter(|f| !f.is_empty())))
}

/// The flake-id shorthand: an id, optionally followed by `/` and a ref
/// and/or revision. Returns `None` when `head` does not have that shape.
/// Once it does, a failure (`nixpkgs/a..b`) is final.
fn parse_flake_id(head: &str) -> Option<NixUriResult<FlakeRef>> {
    let (id, tail) = match head.split_once('/') {
        Some((id, tail)) => (id, Some(tail)),
        None => (head, None),
    };
    if !is_flake_id(id) || !tail.is_none_or(matches_ref_shape) {
        return None;
    }
    let segments: Vec<&str> = head.split('/').filter(|s| !s.is_empty()).collect();
    Some(
        Indirect::from_segments(&segments, head)
            .map(|indirect| FlakeRef::new(FlakeRefType::Indirect(indirect))),
    )
}

enum UrlFailure {
    /// Not a URL, or a malformed one. The input is retried as a path.
    /// Carries the URL error, if there was a scheme to blame.
    Malformed(Option<NixUriError>),
    /// A URL that is rejected outright.
    Final(NixUriError),
}

impl From<NixUriError> for UrlFailure {
    fn from(err: NixUriError) -> Self {
        Self::Malformed(Some(err))
    }
}

fn parse_url(head: &str) -> Result<FlakeRef, UrlFailure> {
    let Some(scheme) = scheme_of(head) else {
        return Err(UrlFailure::Malformed(None));
    };
    let final_err = |err| UrlFailure::Final(err);
    // Nix checks the URL syntax before it looks for an input type, so a malformed URL is
    // malformed even when no input type claims its scheme.
    let unsupported = |reason| match Url::parse(&encoding::mask_non_ascii_escapes(head)) {
        Ok(_) => UrlFailure::Final(NixUriError::Unsupported(reason)),
        Err(_) => UrlFailure::Malformed(Url::parse(head).err()),
    };
    let (application, transport) = split_scheme(scheme);
    enum Scheme {
        Forge(GitForgePlatform),
        Indirect,
        Path,
        Git,
        Mercurial,
        Tarball,
        File,
    }
    let kind = match (application, transport) {
        (None, forge) if let Some(platform) = GitForgePlatform::from_type(forge) => {
            Scheme::Forge(platform)
        }
        (None, "flake") => Scheme::Indirect,
        (None, "path") => Scheme::Path,
        (None, "git") | (Some("git"), _) => Scheme::Git,
        (Some("hg"), t) if HG_TRANSPORTS.contains(&t) => Scheme::Mercurial,
        (Some("tarball") | None, t) if DOWNLOAD_TRANSPORTS.contains(&t) => Scheme::Tarball,
        (Some("file"), t) if DOWNLOAD_TRANSPORTS.contains(&t) => Scheme::File,
        (Some("hg" | "tarball" | "file"), t) => {
            return Err(unsupported(UnsupportedReason::TransportLayer {
                ty: t.into(),
            }));
        }
        _ => {
            return Err(unsupported(UnsupportedReason::UriType {
                ty: scheme.into(),
            }));
        }
    };
    let mut url = Url::parse(head)?;
    let dir = url.query.remove("dir").map(Cow::into_owned);
    let kind = match kind {
        Scheme::Forge(platform) => FlakeRefType::GitForge(parse_git_forge(platform, &url)?),
        Scheme::Indirect => {
            // Every query parameter but `dir` is ignored.
            let segments: Vec<&str> = url.non_empty_segments().collect();
            FlakeRefType::Indirect(Indirect::from_segments(&segments, head)?)
        }
        Scheme::Path => {
            if url.authority.as_ref().is_some_and(Authority::has_host) {
                return Err(final_err(NixUriError::Unsupported(
                    UnsupportedReason::Authority { scheme: "path" },
                )));
            }
            let mut path = PathRef::from_url(&url)?;
            path.apply_query(&url.query).map_err(final_err)?;
            FlakeRefType::Path(path)
        }
        Scheme::Git => FlakeRefType::Git(GitRepo::from_url(url)?),
        Scheme::Mercurial => FlakeRefType::Mercurial(MercurialRepo::from_url(url)?),
        Scheme::Tarball => FlakeRefType::Tarball(Download::from_url(url)),
        Scheme::File => FlakeRefType::File(Download::from_url(url)),
    };
    Ok(FlakeRef::new(kind).with_dir(dir))
}

/// The part of a git-forge URL after `<platform>:`: `<owner>/<repo>[/<ref-or-rev>]`, and
/// its query.
///
/// Empty segments are skipped (`github:/o/r` and `github:o/r/main/` are fine) and every
/// segment after the repo forms the ref (`github:o/r/feature/x` is the ref `feature/x`).
/// Owner and repo are not validated beyond URL syntax. A path segment of 40 hex digits is
/// a revision, anything else a ref.
///
/// The query takes `ref`, `rev` (a SHA-1 hash), `host` and `narHash`. A ref or a revision
/// is given once, and not both.
fn parse_git_forge(platform: GitForgePlatform, url: &Url<'_>) -> Result<GitForge, UrlFailure> {
    let final_err = |err| UrlFailure::Final(err);
    let segments: Vec<&str> = url.non_empty_segments().collect();
    let [owner, repo, rest @ ..] = segments.as_slice() else {
        return Err(
            NixUriError::Unsupported(UnsupportedReason::MissingParameter {
                ty: platform.to_string(),
                parameter: if segments.is_empty() { "owner" } else { "repo" }.to_string(),
            })
            .into(),
        );
    };
    let mut forge = GitForge::new(platform, (*owner).to_string(), (*repo).to_string());
    match rest {
        [] => {}
        [value] if is_sha1_hex(value) => forge.rev = Some((*value).to_string()),
        _ => forge.ref_ = Some(rest.join("/")),
    }
    // Values are checked in Nix's order, after the URL's shape: only a bad `rev` is
    // final, every other failure leaves the input to be read as a path.
    let mut host = None;
    for (key, value) in &url.query {
        match key.as_ref() {
            "narHash" => forge.nar_hash = Some(value.to_string()),
            "host" => host = Some(value),
            "ref" => {
                if forge.ref_.is_some() {
                    return Err(NixUriError::DuplicateField { field: "ref" }.into());
                }
                forge.ref_ = Some(value.to_string());
            }
            "rev" => {
                if forge.rev.is_some() {
                    return Err(NixUriError::DuplicateField { field: "rev" }.into());
                }
                forge.rev = Some(value.to_string());
            }
            _ => {
                return Err(NixUriError::Unsupported(UnsupportedReason::Param {
                    name: key.to_string(),
                })
                .into());
            }
        }
    }
    if forge.ref_.is_some() && forge.rev.is_some() {
        return Err(NixUriError::FieldConflict {
            left: "ref",
            right: "rev",
        }
        .into());
    }
    if forge.rev.as_deref().is_some_and(|rev| !is_sha1_hash(rev)) {
        return Err(final_err(NixUriError::InvalidValue {
            field: "rev",
            reason: "expected a SHA-1 commit hash".to_string(),
        }));
    }
    if let Some(ref_) = &forge.ref_ {
        forge.ref_ = Some(validated_ref_name(ref_)?);
    }
    if let Some(host) = host {
        forge.host = Some(validated_host_name(host)?);
    }
    Ok(forge)
}

/// The last reading: everything before `?` is a path, kept as written apart from runs of
/// `/`. Only absolute paths are accepted in [`Context::Standalone`].
fn parse_path(head: &str, context: Context) -> NixUriResult<FlakeRef> {
    let (path, query) = match head.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (head, None),
    };
    if !path.starts_with('/') && !context.allows_relative_path() {
        let reason = if looks_like_scp_url(path) {
            UnsupportedReason::ScpStyleUrl { url: path.into() }
        } else {
            UnsupportedReason::RelativePath { path: path.into() }
        };
        return Err(NixUriError::Unsupported(reason));
    }
    if path.contains('\0') {
        return Err(nul_in_path(path));
    }
    let mut path = PathRef::from_bare(path);
    let mut query = match query {
        Some(query) => parse_query(query)?,
        None => Default::default(),
    };
    let dir = query.remove("dir").map(Cow::into_owned);
    path.apply_query(&query)?;
    Ok(FlakeRef::new(FlakeRefType::Path(path)).with_dir(dir))
}

/// An SCP-style git location, `user@host:path`, as written by
/// `git clone`. Only used to explain why such an input is rejected.
fn looks_like_scp_url(path: &str) -> bool {
    path.split_once(':').is_some_and(|(authority, rest)| {
        authority.contains('@') && !authority.contains('/') && !rest.is_empty()
    })
}

#[cfg(test)]
mod ref_name_validation {
    //! Public-surface coverage for parse-time `ref` validation. The ref-name shape is
    //! enforced at every routing site that hands a string into a kind's typed `ref_`
    //! slot. These tests pin the user-visible behaviour of the rejection paths.
    use crate::{Context, FlakeRef, NixUriError};
    use std::assert_matches;

    #[test]
    fn query_ref_rejects_whitespace() {
        assert_matches!(
            "github:o/r?ref=invalid ref".parse::<FlakeRef>(),
            Err(NixUriError::InvalidValue { field: "ref", .. })
        );
    }

    #[test]
    fn query_ref_rejects_leading_dash() {
        assert_matches!(
            "github:o/r?ref=-leading-dash".parse::<FlakeRef>(),
            Err(NixUriError::InvalidValue { field: "ref", .. })
        );
    }

    #[test]
    fn path_component_ref_rejects_leading_dot() {
        // GitHub path-component ref: `github:o/r/.hidden`. A flake input
        // would read the malformed URL as a relative path.
        assert_matches!(
            FlakeRef::parse("github:o/r/.hidden", Context::Standalone),
            Err(NixUriError::InvalidValue { field: "ref", .. })
        );
    }

    #[test]
    fn indirect_ref_rejects_leading_dash() {
        assert_matches!(
            FlakeRef::parse("flake:nixpkgs/-bad", Context::Standalone),
            Err(NixUriError::InvalidValue { field: "ref", .. })
        );
    }

    #[test]
    fn query_ref_accepts_namespaced_branch() {
        let parsed: FlakeRef = "github:o/r?ref=feature/foo"
            .parse()
            .expect("namespaced branch must parse");
        assert_eq!(parsed.to_string(), "github:o/r/feature%2Ffoo");
    }

    #[test]
    fn query_ref_accepts_release_branch() {
        let parsed: FlakeRef = "github:o/r?ref=release-23.05"
            .parse()
            .expect("release branch must parse");
        assert_eq!(parsed.to_string(), "github:o/r/release-23.05");
    }
}

#[cfg(test)]
mod host_name_validation {
    //! Public-surface coverage for parse-time `?host=` validation. The accepted host-attr
    //! shape (`[a-zA-Z0-9.-]*`) is enforced at the param-routing site, and values outside
    //! it surface here as `InvalidValue`.
    use crate::{FlakeRef, NixUriError};
    use rstest::rstest;
    use std::assert_matches;

    #[rstest]
    #[case("github:o/r?host=bad!host")]
    #[case("github:o/r?host=space host")]
    #[case("github:o/r?host=ho'st")]
    // Underscore is outside the `[a-zA-Z0-9.-]*` host shape. The pre-validator generator would
    // happily produce it on a github URL, so pin the rejection at parse time too.
    #[case("github:o/r?host=under_score")]
    fn host_with_invalid_chars_rejects(#[case] uri: &str) {
        assert_matches!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::InvalidValue { field: "host", .. })
        );
    }

    #[rstest]
    #[case("github:o/r?host=localhost")]
    #[case("gitlab:o/r?host=git.openldap.org")]
    #[case("github:o/r?host=1.2.3.4")]
    fn host_with_valid_chars_accepts(#[case] uri: &str) {
        let parsed: FlakeRef = uri.parse().expect("valid host must parse");
        assert_eq!(parsed.to_string(), uri);
    }
}

#[cfg(test)]
mod rev_validation {
    //! Public-surface coverage for parse-time git-forge `?rev=`
    //! validation. It must be a SHA-1 hash, and anything else
    //! surfaces as `InvalidValue { field: "rev", .. }`.
    use crate::{FlakeRef, NixUriError};
    use std::assert_matches;

    #[test]
    fn rev_query_40_hex_accepted() {
        let parsed: FlakeRef = "github:o/r?rev=b2df4e4e80e04cbb33a350f87717f4bd6140d298"
            .parse()
            .expect("40-hex rev (SHA-1) must parse");
        assert_eq!(
            parsed.to_string(),
            "github:o/r/b2df4e4e80e04cbb33a350f87717f4bd6140d298"
        );
    }

    #[test]
    fn rev_query_64_hex_rejected() {
        // A git forge revision is a SHA-1 hash.
        assert_matches!(
            "github:o/r?rev=0000000000000000000000000000000000000000000000000000000000000000"
                .parse::<FlakeRef>(),
            Err(NixUriError::InvalidValue { field: "rev", .. })
        );
    }

    #[test]
    fn rev_query_41_hex_rejected() {
        assert_matches!(
            "github:o/r?rev=b2df4e4e80e04cbb33a350f87717f4bd6140d2980".parse::<FlakeRef>(),
            Err(NixUriError::InvalidValue { field: "rev", .. })
        );
    }

    #[test]
    fn rev_query_63_hex_rejected() {
        assert_matches!(
            "github:o/r?rev=000000000000000000000000000000000000000000000000000000000000000"
                .parse::<FlakeRef>(),
            Err(NixUriError::InvalidValue { field: "rev", .. })
        );
    }

    #[test]
    fn rev_query_50_hex_rejected() {
        assert_matches!(
            "github:o/r?rev=00000000000000000000000000000000000000000000000000".parse::<FlakeRef>(),
            Err(NixUriError::InvalidValue { field: "rev", .. })
        );
    }

    #[test]
    fn rev_path_component_64_hex_is_a_ref() {
        // Only 40 hex digits are a revision in a path segment. Nix 2.35.3 reads
        // `github:o/r/<64 hex>` as the ref of that name.
        let hex = "0000000000000000000000000000000000000000000000000000000000000000";
        let parsed: FlakeRef = format!("github:o/r/{hex}").parse().unwrap();
        assert_eq!(parsed.ref_(), Some(hex));
        assert_eq!(parsed.rev(), None);
    }
}

#[cfg(test)]
mod scp_style {
    //! SCP-style git locations (`[<user>@]<host>:<path>`), as written by
    //! `git clone`, are not flake references. `builtins.parseFlakeRef`
    //! rejects them, and a flake input reads them as relative paths, like
    //! any other string that is not a URL.
    use crate::{Context, FlakeRef, FlakeRefType, NixUriError, PathRef, UnsupportedReason};
    use rstest::rstest;
    use std::assert_matches;

    #[rstest]
    #[case("git@github.com:nixos/nixpkgs")]
    #[case("git@gitlab.com:owner/repo")]
    #[case("git@git.sr.ht:~user/proj")]
    #[case("git@github.com:owner/repo/")]
    #[case("first.last@host.example:team/svc")]
    #[case("git@host.example:/srv/git/repo.git")]
    #[case("user@[::1]:repo.git")]
    #[case("git+ssh://git@github.com:nixos/nixpkgs")]
    fn scp_style_location(#[case] input: &str) {
        assert_matches!(
            FlakeRef::parse(input, Context::Standalone),
            Err(NixUriError::Unsupported(
                UnsupportedReason::ScpStyleUrl { .. }
            )),
            "{input}"
        );
        // Runs of `/` collapse, as in every path.
        let parsed = FlakeRef::parse(input, Context::FlakeInput).unwrap();
        assert_eq!(
            parsed.kind,
            FlakeRefType::Path(PathRef::new(input.replace("//", "/")))
        );
    }

    #[rstest]
    fn scp_style_without_user_is_an_unknown_scheme(
        #[values(Context::Standalone, Context::FlakeInput)] context: Context,
    ) {
        // `github.com` is a valid URL scheme, just not a known one.
        assert_matches!(
            FlakeRef::parse("github.com:nixos/nixpkgs", context),
            Err(NixUriError::Unsupported(UnsupportedReason::UriType { .. }))
        );
    }

    #[test]
    fn ssh_url_with_port_is_not_scp_style() {
        let uri = "git+ssh://git@github.com:22/nixos/nixpkgs";
        let parsed = FlakeRef::parse(uri, Context::Standalone).unwrap();
        assert_eq!(parsed.to_string(), uri);
    }
}

#[cfg(test)]
mod arbitrary_query_keys {
    //! Git forges and `path:` only take the query keys their fetcher knows. Git,
    //! Mercurial, tarball and file URLs keep unknown keys in their `url`, and `flake:`
    //! ignores its query, see the Nix tables in `nix_acceptance`.
    use crate::{Context, FlakeRef, NixUriError, UnsupportedReason};
    use rstest::rstest;
    use std::assert_matches;

    #[rstest]
    #[case::github("github:o/r?treeHash=abc")]
    #[case::github_submodules("github:o/r?submodules=1")]
    #[case::github_last_modified("github:o/r?lastModified=1")]
    #[case::gitlab("gitlab:o/r?wurzel=pfropf")]
    #[case::sourcehut("sourcehut:~o/r?treeHash=abc")]
    #[case::path("path:/foo/bar?host=x")]
    #[case::path_ref("path:/foo/bar?ref=main")]
    fn unknown_query_key_rejected_by_forges_and_paths(#[case] input: &str) {
        assert_matches!(
            FlakeRef::parse(input, Context::Standalone),
            Err(NixUriError::Unsupported(UnsupportedReason::Param { .. }))
        );
    }

    /// A malformed escape anywhere in the query is fatal, even in a pair that would be
    /// ignored and on the path reading of a flake input.
    #[rstest]
    #[case::url("github:o/r?a%")]
    #[case::absolute_path("/abs?a%")]
    #[case::relative_path("./x?a%")]
    #[case::fallback_path("h/fla.ke?reg+%")]
    #[case::second_pair("https://h/x?%2p&b=1")]
    fn malformed_query_escape_is_rejected(#[case] input: &str) {
        assert_matches!(
            FlakeRef::parse(input, Context::FlakeInput),
            Err(NixUriError::InvalidUrl(_))
        );
    }
}

#[cfg(test)]
mod nul_bytes {
    use crate::{Context, FlakeRef, FlakeRefType, NixUriError, PathRef};
    use rstest::rstest;
    use std::assert_matches;

    /// A NUL byte in the path of a path reference is rejected, whichever reading produced
    /// it.
    #[rstest]
    #[case::absolute("/a\0b", Context::Standalone)]
    #[case::absolute_with_query("/a\0b?dir=x", Context::Standalone)]
    #[case::last_segment("/a/b\0", Context::FlakeInput)]
    #[case::relative("./a\0", Context::FlakeInput)]
    #[case::fallback("github:o/r\0x", Context::FlakeInput)]
    fn nul_byte_in_a_path_is_rejected(#[case] input: &str, #[case] context: Context) {
        assert_matches!(
            FlakeRef::parse(input, context),
            Err(NixUriError::InvalidUrl(_))
        );
    }

    /// A `path:` URL whose path decodes to a NUL is malformed, so a flake input reads the
    /// whole string as a relative path, and on its own it is not absolute.
    #[test]
    fn path_url_with_an_escaped_nul_falls_back_to_a_relative_path() {
        assert_eq!(
            FlakeRef::parse("path:/a%00b", Context::FlakeInput)
                .unwrap()
                .kind,
            FlakeRefType::Path(PathRef::new("path:/a%00b".into()))
        );
        assert!(FlakeRef::parse("path:/a%00b", Context::Standalone).is_err());
    }

    /// Outside a path a NUL is just another byte.
    #[rstest]
    #[case("github:o/r%00")]
    #[case("https://h/a%00b")]
    #[case("git+file:///a%00b")]
    #[case("github:o/r?dir=a%00")]
    #[case("/a%00")]
    fn escaped_nul_outside_a_path_is_kept(#[case] input: &str) {
        assert!(
            FlakeRef::parse(input, Context::Standalone).is_ok(),
            "{input}"
        );
    }
}

#[cfg(test)]
mod nix_acceptance;
