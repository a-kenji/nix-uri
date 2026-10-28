use crate::{
    Context,
    error::{NixUriError, NixUriResult, UnsupportedReason},
    flakeref::{
        FlakeRef, FlakeRefType, GitForge, LocationParamKeys, LocationParameters, RefLocation,
        ResourceType, encoding,
        fr_type::{indirect_from_segments, is_flake_id},
        location_params::ParamRefRev,
        validators::{
            is_sha1_hash, matches_ref_shape, query_bool, validated_host_name, validated_ref_name,
        },
    },
};

/// Parse a flake reference in `context`.
///
/// Three readings are tried in order:
///
/// 1. the flake-id shorthand `id[/ref-or-rev]` (`nixpkgs/nixos-25.05`),
/// 2. a URL with a scheme (`github:`, `git+https:`, `path:`, ...),
/// 3. a path, with an optional query.
///
/// A URL that is malformed falls through to the path reading, which in a
/// [`Context::FlakeInput`] keeps it as a relative path
/// (`github:NixOS/nixpkgs/a..b` is the directory of that name). Other URL
/// failures, such as an unknown scheme, are final.
pub(crate) fn parse_nix_uri(input: &str, context: Context) -> NixUriResult<FlakeRef> {
    if input.trim().is_empty() || input.chars().any(char::is_control) {
        return Err(NixUriError::InvalidUrl(input.into()));
    }

    let (head, fragment) = match input.split_once('#') {
        Some((head, fragment)) => (head, Some(encoding::decode_percent(fragment)?.into_owned())),
        None => (input, None),
    };

    let mut flake_ref = match parse_flake_id(head) {
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

    if let Some(fragment) = fragment.as_deref().filter(|f| !f.is_empty())
        && !context.allows_fragment()
    {
        return Err(NixUriError::Unsupported(UnsupportedReason::Fragment {
            fragment: fragment.into(),
        }));
    }
    flake_ref.set_fragment(fragment);
    Ok(flake_ref)
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
    Some(indirect_from_segments(&segments, head).map(FlakeRef::new))
}

enum UrlFailure {
    /// Not a URL, or a malformed one. The input is retried as a path.
    /// Carries the URL error, if there was a scheme to blame.
    Malformed(Option<NixUriError>),
    /// A URL that is rejected outright.
    Final(NixUriError),
}

fn parse_url(head: &str) -> Result<FlakeRef, UrlFailure> {
    let Some(scheme) = url_scheme(head) else {
        return Err(UrlFailure::Malformed(None));
    };
    let (location, query) = split_query(head);
    let parsed = FlakeRefType::parse_type(location).and_then(|kind| {
        let mut flake_ref = FlakeRef::new(kind);
        apply_query(&mut flake_ref, query)?;
        Ok(flake_ref)
    });
    parsed.map_err(|err| {
        if is_final_url_error(scheme, &err) {
            UrlFailure::Final(err)
        } else {
            UrlFailure::Malformed(Some(err))
        }
    })
}

/// The scheme of `input`, if it starts with one: a letter followed by
/// letters, digits, `+`, `-` or `.`, then `:`.
fn url_scheme(input: &str) -> Option<&str> {
    let (scheme, _) = input.split_once(':')?;
    let mut chars = scheme.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    valid.then_some(scheme)
}

/// A URL is only retried as a path when it is malformed. An unknown
/// scheme or transport, a `path:` URL with an authority or a bad
/// parameter, and a git-forge `?rev=` that is not a SHA-1 hash are final.
fn is_final_url_error(scheme: &str, err: &NixUriError) -> bool {
    match err {
        NixUriError::Unsupported(
            UnsupportedReason::UriType { .. }
            | UnsupportedReason::TransportLayer { .. }
            | UnsupportedReason::Authority { .. },
        ) => true,
        NixUriError::Unsupported(UnsupportedReason::Param { .. })
        | NixUriError::InvalidValue { .. }
            if scheme == "path" =>
        {
            true
        }
        NixUriError::InvalidValue { field: "rev", .. } => {
            matches!(scheme, "github" | "gitlab" | "sourcehut")
        }
        _ => false,
    }
}

/// The last reading: everything before `?` is a path. Only absolute paths
/// are accepted in [`Context::Standalone`].
fn parse_path(head: &str, context: Context) -> NixUriResult<FlakeRef> {
    let (path, query) = split_query(head);
    if path.is_empty() {
        return Err(NixUriError::InvalidUrl(head.into()));
    }
    if !path.starts_with('/') && !context.allows_relative_path() {
        let reason = if looks_like_scp_url(path) {
            UnsupportedReason::ScpStyleUrl { url: path.into() }
        } else {
            UnsupportedReason::RelativePath { path: path.into() }
        };
        return Err(NixUriError::Unsupported(reason));
    }
    let mut flake_ref = FlakeRef::new(FlakeRefType::Path {
        path: path.into(),
        rev: None,
    });
    apply_query(&mut flake_ref, query)?;
    Ok(flake_ref)
}

/// An SCP-style git location, `user@host:path`, as written by
/// `git clone`. Only used to explain why such an input is rejected.
fn looks_like_scp_url(path: &str) -> bool {
    path.split_once(':').is_some_and(|(authority, rest)| {
        authority.contains('@') && !authority.contains('/') && !rest.is_empty()
    })
}

fn split_query(input: &str) -> (&str, Option<&str>) {
    match input.split_once('?') {
        Some((before, query)) => (before, Some(query)),
        None => (input, None),
    }
}

/// Parse `query`, check it against what the kind of `flake_ref` allows,
/// and store it on `flake_ref`.
fn apply_query(flake_ref: &mut FlakeRef, query: Option<&str>) -> NixUriResult<()> {
    let Some(query) = query else {
        return Ok(());
    };
    let pairs = parse_query(query)?;
    check_query(flake_ref.kind(), &pairs)?;
    let (params, ref_rev) = route_location_params(pairs);
    flake_ref.replace_params(params);
    apply_param_ref_rev(flake_ref, ref_rev)
}

/// Split a query into `&`-separated `key=value` pairs, both
/// sides percent-decoded. A pair without `=` is ignored, and of repeated
/// keys the first one wins.
pub(crate) fn parse_query(query: &str) -> NixUriResult<Vec<(String, String)>> {
    if !query.chars().all(encoding::is_query_char) {
        return Err(NixUriError::InvalidUrl(query.into()));
    }
    let mut pairs: Vec<(String, String)> = Vec::new();
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let key = encoding::decode_percent(key)?.into_owned();
        if pairs.iter().any(|(seen, _)| *seen == key) {
            continue;
        }
        let value = encoding::decode_percent(value)?.into_owned();
        pairs.push((key, value));
    }
    Ok(pairs)
}

/// Check each query parameter against what the kind accepts.
///
/// `dir` is the flake's subdirectory and allowed everywhere. Git forges
/// take only `ref`, `rev` (a SHA-1 hash), `host` and `narHash`. `path:`
/// takes `rev`, `narHash` and the integers `revCount` and `lastModified`.
/// Git checks `ref` against Git's ref-name rules and Mercurial against
/// the flake-id ref shape. Everything else is passed through to the
/// fetcher and is not checked, and `flake:` ignores its query entirely.
fn check_query(kind: &FlakeRefType, query: &[(String, String)]) -> NixUriResult<()> {
    for (key, value) in query {
        let unsupported = || {
            Err(NixUriError::Unsupported(UnsupportedReason::Param {
                name: key.clone(),
            }))
        };
        match kind {
            FlakeRefType::GitForge(_) => match key.as_str() {
                "dir" | "narHash" => {}
                "ref" => {
                    validated_ref_name(value)?;
                }
                "rev" => require_sha1_hash(value)?,
                "host" => {
                    validated_host_name(value)?;
                }
                _ => return unsupported(),
            },
            FlakeRefType::Path { .. } => match key.as_str() {
                "dir" | "rev" | "narHash" => {}
                "revCount" => require_integer("revCount", value)?,
                "lastModified" => require_integer("lastModified", value)?,
                _ => return unsupported(),
            },
            FlakeRefType::Resource(res) if key == "ref" => match res.res_type {
                ResourceType::Git => {
                    validated_ref_name(value)?;
                }
                ResourceType::Mercurial if !matches_ref_shape(value) => {
                    return Err(NixUriError::InvalidValue {
                        field: "ref",
                        reason: "expected a Mercurial branch or tag name".to_string(),
                    });
                }
                _ => {}
            },
            FlakeRefType::Resource(_) | FlakeRefType::Indirect { .. } => {}
        }
    }
    Ok(())
}

fn require_sha1_hash(value: &str) -> NixUriResult<()> {
    if is_sha1_hash(value) {
        return Ok(());
    }
    Err(NixUriError::InvalidValue {
        field: "rev",
        reason: "expected a SHA-1 commit hash".to_string(),
    })
}

fn require_integer(field: &'static str, value: &str) -> NixUriResult<()> {
    if value.parse::<u64>().is_ok() {
        return Ok(());
    }
    Err(NixUriError::InvalidValue {
        field,
        reason: "expected a non-negative integer".to_string(),
    })
}

/// Route decoded query pairs into their typed destinations: the slots on
/// [`LocationParameters`], and the `ref` / `rev` side-channel
/// ([`ParamRefRev`]) whose values belong on the kind. Keys without a typed
/// slot (e.g. `name`, `treeHash`) round-trip through
/// `LocationParameters::arbitrary`. A boolean parameter is true only for
/// `"1"`.
pub(crate) fn route_location_params(
    pairs: Vec<(String, String)>,
) -> (LocationParameters, ParamRefRev) {
    let mut params = LocationParameters::default();
    let mut ref_rev = ParamRefRev::default();
    for (key, value) in pairs {
        match LocationParamKeys::from(key.as_str()) {
            LocationParamKeys::Dir => params.set_dir(Some(value)),
            LocationParamKeys::NarHash => params.set_nar_hash(Some(value)),
            LocationParamKeys::LastModified => params.set_last_modified(Some(value)),
            LocationParamKeys::RevCount => params.set_rev_count(Some(value)),
            LocationParamKeys::Host => params.set_host(Some(value)),
            LocationParamKeys::Ref => ref_rev.r#ref = Some(value),
            LocationParamKeys::Rev => ref_rev.rev = Some(value),
            LocationParamKeys::Submodules => params.set_submodules(Some(query_bool(&value))),
            LocationParamKeys::Shallow => params.set_shallow(Some(query_bool(&value))),
            LocationParamKeys::Lfs => params.set_lfs(Some(query_bool(&value))),
            LocationParamKeys::ExportIgnore => {
                params.set_export_ignore(Some(query_bool(&value)));
            }
            LocationParamKeys::AllRefs => params.set_all_refs(Some(query_bool(&value))),
            LocationParamKeys::VerifyCommit => {
                params.set_verify_commit(Some(query_bool(&value)));
            }
            LocationParamKeys::Keytype => params.set_keytype(Some(value)),
            LocationParamKeys::PublicKey => params.set_public_key(Some(value)),
            LocationParamKeys::PublicKeys => params.set_public_keys(Some(value)),
            LocationParamKeys::Arbitrary(k) => params.add_arbitrary((k, value)),
        }
    }
    (params, ref_rev)
}

/// Apply a [`ParamRefRev`] to a [`FlakeRef`]: write the ref/rev into the
/// kind's typed slots and mark the kind's [`RefLocation`] as
/// `QueryParameter` so a Display round-trip preserves the `?ref=` / `?rev=`
/// form. No-op if both slots are empty. The values were already checked by
/// [`check_query`].
///
/// A git forge takes a ref or a revision, from the path or from the query,
/// but not both, and not the same one twice. So `github:o/r/main?ref=dev`
/// and `github:o/r/main?rev=<hash>` are rejected.
///
/// `Path` has no typed `ref_` slot, and `?ref=` is rejected for it before
/// this point.
pub(crate) fn apply_param_ref_rev(
    flake_ref: &mut FlakeRef,
    ref_rev: ParamRefRev,
) -> NixUriResult<()> {
    if ref_rev.r#ref.is_none() && ref_rev.rev.is_none() {
        return Ok(());
    }
    if let FlakeRefType::GitForge(GitForge { ref_, rev, .. }) = flake_ref.kind() {
        if ref_.is_some() && ref_rev.r#ref.is_some() {
            return Err(NixUriError::DuplicateField { field: "ref" });
        }
        if rev.is_some() && ref_rev.rev.is_some() {
            return Err(NixUriError::DuplicateField { field: "rev" });
        }
        if ref_.is_some() || rev.is_some() || (ref_rev.r#ref.is_some() && ref_rev.rev.is_some()) {
            return Err(NixUriError::FieldConflict {
                left: "ref",
                right: "rev",
            });
        }
    }
    if ref_rev.r#ref.is_some() {
        flake_ref.kind_mut().set_ref(ref_rev.r#ref);
    }
    if ref_rev.rev.is_some() {
        flake_ref.kind_mut().set_rev(ref_rev.rev);
    }
    flake_ref
        .kind_mut()
        .set_ref_location(RefLocation::QueryParameter);
    Ok(())
}

pub(crate) fn is_tarball(input: &str) -> bool {
    let valid_extensions = &[
        ".zip", ".tar", ".tgz", ".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst",
    ];
    valid_extensions.iter().any(|&ext| input.ends_with(ext))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn check_tarball() {
        let filename = "example.tar.gz";
        assert!(is_tarball(filename));
    }
    #[test]
    fn check_tarball_uri() {
        let filename = "https://github.com/NixOS/patchelf/archive/master.tar.gz";
        assert!(is_tarball(filename));
    }
    #[test]
    fn check_file_uri() {
        let filename = "https://github.com/NixOS/patchelf/";
        assert!(!is_tarball(filename));
    }
    #[test]
    fn check_file() {
        let filename = "example";
        assert!(!is_tarball(filename));
    }
}

#[cfg(test)]
mod ref_name_validation {
    //! Public-surface coverage for parse-time `ref` validation. Nix's
    //! ref-name shape is enforced at every routing site that hands a
    //! string into a kind's typed `ref_` slot; these tests pin the
    //! user-visible behaviour of the rejection paths.
    use crate::{Context, FlakeRef, NixUriError};
    use cool_asserts::assert_matches;

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
        assert_eq!(parsed.to_string(), "github:o/r?ref=feature/foo");
    }

    #[test]
    fn query_ref_accepts_release_branch() {
        let parsed: FlakeRef = "github:o/r?ref=release-23.05"
            .parse()
            .expect("release branch must parse");
        assert_eq!(parsed.to_string(), "github:o/r?ref=release-23.05");
    }
}

#[cfg(test)]
mod host_name_validation {
    //! Public-surface coverage for parse-time `?host=` validation. Nix's
    //! accepted host-attr shape (`[a-zA-Z0-9.-]*`) is enforced at the
    //! param-routing site; values outside it surface here as
    //! `InvalidValue`.
    use crate::{FlakeRef, NixUriError};
    use cool_asserts::assert_matches;
    use rstest::rstest;

    #[rstest]
    #[case("github:o/r?host=bad!host")]
    #[case("github:o/r?host=space host")]
    #[case("github:o/r?host=ho'st")]
    // Underscore is outside Nix's `[a-zA-Z0-9.-]*` host shape; the
    // pre-validator generator would happily produce it on a github
    // URL, so pin the rejection at parse time too.
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
    use cool_asserts::assert_matches;

    #[test]
    fn rev_query_40_hex_accepted() {
        let parsed: FlakeRef = "github:o/r?rev=b2df4e4e80e04cbb33a350f87717f4bd6140d298"
            .parse()
            .expect("40-hex rev (SHA-1) must parse");
        assert_eq!(
            parsed.to_string(),
            "github:o/r?rev=b2df4e4e80e04cbb33a350f87717f4bd6140d298"
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
    fn rev_path_component_64_hex_classifies_as_rev() {
        // The `looks_like_rev` classifier is also what splits a single
        // path-component value (`github:o/r/<x>`) between `ref_` and `rev`.
        // A 64-hex segment must classify as `rev`.
        let parsed: FlakeRef =
            "github:o/r/0000000000000000000000000000000000000000000000000000000000000000"
                .parse()
                .expect("64-hex path-component must parse as rev");
        match parsed.kind() {
            crate::FlakeRefType::GitForge(g) => {
                assert!(g.ref_.is_none(), "ref_ must be empty");
                assert_eq!(
                    g.rev.as_deref(),
                    Some("0000000000000000000000000000000000000000000000000000000000000000")
                );
            }
            other => panic!("expected GitForge, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod scp_style {
    //! SCP-style git locations (`[<user>@]<host>:<path>`), as written by
    //! `git clone`, are not flake references. `builtins.parseFlakeRef`
    //! rejects them, and a flake input reads them as relative paths, like
    //! any other string that is not a URL.
    use crate::{Context, FlakeRef, FlakeRefType, NixUriError, UnsupportedReason};
    use cool_asserts::assert_matches;
    use rstest::rstest;

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
        let parsed = FlakeRef::parse(input, Context::FlakeInput).unwrap();
        assert_eq!(
            parsed.kind,
            FlakeRefType::Path {
                path: input.to_string(),
                rev: None,
            }
        );
    }

    #[test]
    fn scp_style_without_user_is_an_unknown_scheme() {
        // `github.com` is a valid URL scheme, just not a known one.
        for context in [Context::Standalone, Context::FlakeInput] {
            assert_matches!(
                FlakeRef::parse("github.com:nixos/nixpkgs", context),
                Err(NixUriError::Unsupported(UnsupportedReason::UriType { .. }))
            );
        }
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
    //! Public-surface coverage for which query keys each scheme takes.
    //! Git, Mercurial, tarball and file URLs pass unknown keys on to the
    //! fetcher, and `flake:` ignores its query, so every key is accepted
    //! there. Keys without a typed slot land verbatim in
    //! [`crate::LocationParameters`]'s arbitrary bag. Git forges and
    //! `path:` only take the keys their fetcher knows.
    use crate::{Context, FlakeRef, NixUriError, UnsupportedReason};
    use cool_asserts::assert_matches;
    use rstest::rstest;

    fn assert_value_preserved(input: &str, key: &str, value: &str) {
        let parsed = FlakeRef::parse(input, Context::Standalone)
            .unwrap_or_else(|e| panic!("{input}: expected success, got {e:?}"));
        let rendered = parsed.to_string();
        let needle = format!("{key}={value}");
        assert!(
            rendered.contains(&needle),
            "{input}: `{needle}` missing in Display output `{rendered}`",
        );
    }

    #[rstest]
    #[case::indirect("flake:nixpkgs?host=foo", "host", "foo")]
    #[case::git("git+ssh://example.com/repo?treeHash=abc", "treeHash", "abc")]
    #[case::mercurial("hg+https://example.com/repo?lfs=1", "lfs", "1")]
    #[case::tarball_ref("tarball+https://example.com/x.tar.gz?ref=v1", "ref", "v1")]
    #[case::file_ref("file+https://example.com/x?ref=v1", "ref", "v1")]
    #[case::tarball("tarball+https://example.com/x.tar.gz?lfs=1", "lfs", "1")]
    #[case::file("file+https://example.com/x?lfs=1", "lfs", "1")]
    fn unknown_query_key_routes_to_arbitrary_or_typed_slot(
        #[case] input: &str,
        #[case] key: &str,
        #[case] value: &str,
    ) {
        // Recognised typed keys (`submodules`, `lfs`, `host`, `ref`, ...)
        // route into their typed slots, and keys with no typed slot
        // (`treeHash`) land in the arbitrary bag. Either way Display
        // preserves the `key=value` pair the user wrote.
        assert_value_preserved(input, key, value);
    }

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

    /// The locked attributes `narHash`, `lastModified` and `revCount`, as a
    /// `flake.lock` stores them. Every scheme except the git forges takes
    /// all three in the URL. Git forges only take `narHash` (see
    /// [`unknown_query_key_rejected_by_forges_and_paths`]).
    #[rstest]
    #[case::indirect("flake:nixpkgs?")]
    #[case::git("git+ssh://example.com/repo?")]
    #[case::mercurial("hg+https://example.com/repo?")]
    #[case::path("path:/foo/bar?")]
    #[case::tarball("tarball+https://example.com/x.tar.gz?")]
    #[case::file("file+https://example.com/x?")]
    fn locked_attrs_accepted(#[case] prefix: &str) {
        let input = format!("{prefix}lastModified=1&narHash=sha256-abc&revCount=2");
        let parsed = FlakeRef::parse(&input, Context::Standalone)
            .unwrap_or_else(|e| panic!("{input}: expected success, got {e:?}"));
        // The query string Display sorts keys alphabetically and pulls
        // populated typed slots ahead of the arbitrary bag, so the
        // locked-attrs trio appearing in the rendered output proves the
        // values reached the typed slots rather than `arbitrary`.
        let rendered = parsed.to_string();
        assert!(
            rendered.contains("lastModified=1"),
            "{input}: lastModified missing in {rendered}",
        );
        assert!(
            rendered.contains("narHash=sha256-abc"),
            "{input}: narHash missing in {rendered}",
        );
        assert!(
            rendered.contains("revCount=2"),
            "{input}: revCount missing in {rendered}",
        );
    }
}

#[cfg(test)]
mod nix_acceptance {
    //! What Nix accepts, per [`Context`]. Every row was checked against
    //! Nix 2.35.3: `builtins.parseFlakeRef` for [`Context::Standalone`], a
    //! flake input url for [`Context::FlakeInput`], and the flake input
    //! behaviour plus a fragment for [`Context::Installable`].
    //!
    //! Accepted rows pin the key fields (through [`describe`]) and the
    //! `Display` output. Some newly accepted shapes do not round-trip
    //! byte for byte (`github:/o/r` renders `github:o/r`), so the
    //! expected `Display` is spelled out per row.
    use crate::{
        Context, FlakeRef, FlakeRefType, NixUriError, UnsupportedReason,
        flakeref::{GitForge, ResourceUrl},
    };
    use rstest::rstest;

    const REV: &str = "0000000000000000000000000000000000000000";

    #[derive(Debug, Clone, Copy)]
    enum Rejected {
        Fragment,
        RelativePath,
        ScpStyleUrl,
        Param,
        InvalidRef,
        InvalidRev,
        DuplicateRef,
        UriType,
        /// A malformed URL that is not one of the specific shapes above.
        Malformed,
    }

    impl Rejected {
        fn matches(self, err: &NixUriError) -> bool {
            match self {
                Self::Fragment => matches!(
                    err,
                    NixUriError::Unsupported(UnsupportedReason::Fragment { .. })
                ),
                Self::RelativePath => matches!(
                    err,
                    NixUriError::Unsupported(UnsupportedReason::RelativePath { .. })
                ),
                Self::ScpStyleUrl => matches!(
                    err,
                    NixUriError::Unsupported(UnsupportedReason::ScpStyleUrl { .. })
                ),
                Self::Param => matches!(
                    err,
                    NixUriError::Unsupported(UnsupportedReason::Param { .. })
                ),
                Self::InvalidRef => matches!(err, NixUriError::InvalidValue { field: "ref", .. }),
                Self::InvalidRev => matches!(err, NixUriError::InvalidValue { field: "rev", .. }),
                Self::DuplicateRef => matches!(err, NixUriError::DuplicateField { field: "ref" }),
                Self::UriType => matches!(
                    err,
                    NixUriError::Unsupported(UnsupportedReason::UriType { .. })
                ),
                Self::Malformed => true,
            }
        }
    }

    /// The fields a row pins, in a compact `kind key=value ...` form.
    fn describe(flake_ref: &FlakeRef) -> String {
        let mut out = match flake_ref.kind() {
            FlakeRefType::GitForge(GitForge {
                platform,
                owner,
                repo,
                ..
            }) => format!("{platform} owner={owner} repo={repo}"),
            FlakeRefType::Indirect { id, .. } => format!("indirect id={id}"),
            FlakeRefType::Path { path, .. } => format!("path {path}"),
            FlakeRefType::Resource(ResourceUrl {
                res_type,
                transport_type,
                location,
                ..
            }) => {
                let transport = transport_type
                    .as_ref()
                    .map_or_else(|| "-".to_string(), ToString::to_string);
                format!("{res_type} {transport} {location}")
            }
        };
        if let Some(r) = flake_ref.ref_() {
            out.push_str(&format!(" ref={r}"));
        }
        if let Some(r) = flake_ref.rev() {
            out.push_str(&format!(" rev={r}"));
        }
        if let Some(s) = flake_ref.params().submodules {
            out.push_str(&format!(" submodules={s}"));
        }
        if let Some(f) = flake_ref.fragment() {
            out.push_str(&format!(" #{f}"));
        }
        out
    }

    fn accepts(input: &str, context: Context, fields: &str, display: &str) {
        let parsed = FlakeRef::parse(input, context)
            .unwrap_or_else(|e| panic!("{input:?} in {context:?}: expected accept, got {e:?}"));
        assert_eq!(describe(&parsed), fields, "{input:?} in {context:?}");
        assert_eq!(parsed.to_string(), display, "{input:?} in {context:?}");
    }

    fn rejects(input: &str, context: Context, kind: Rejected) {
        match FlakeRef::parse(input, context) {
            Ok(parsed) => panic!("{input:?} in {context:?}: expected {kind:?}, got {parsed:?}"),
            Err(err) => assert!(
                kind.matches(&err),
                "{input:?} in {context:?}: expected {kind:?}, got {err:?}"
            ),
        }
    }

    /// Accepted the same way in every context.
    #[rstest]
    #[case(
        "github:ncaq/.xmonad",
        "github owner=ncaq repo=.xmonad",
        "github:ncaq/.xmonad"
    )]
    #[case(
        "github:/InioX/Matugen",
        "github owner=InioX repo=Matugen",
        "github:InioX/Matugen"
    )]
    #[case("github:o//r", "github owner=o repo=r", "github:o/r")]
    #[case("github:-o/r", "github owner=-o repo=r", "github:-o/r")]
    #[case("github:o%2Fsub/r", "github owner=o/sub repo=r", "github:o%2Fsub/r")]
    #[case("github:~o/r", "github owner=~o repo=r", "github:~o/r")]
    #[case("sourcehut:o/r", "sourcehut owner=o repo=r", "sourcehut:o/r")]
    #[case(
        "github:NixOS/nixpkgs/nixos-25.05/",
        "github owner=NixOS repo=nixpkgs ref=nixos-25.05",
        "github:NixOS/nixpkgs/nixos-25.05"
    )]
    #[case(
        "github:o/r//main",
        "github owner=o repo=r ref=main",
        "github:o/r/main"
    )]
    #[case("github:o/r/a/b", "github owner=o repo=r ref=a/b", "github:o/r/a/b")]
    #[case("github:o/r?ref", "github owner=o repo=r", "github:o/r")]
    #[case(
        "github:o/r?&ref=main",
        "github owner=o repo=r ref=main",
        "github:o/r?ref=main"
    )]
    #[case(
        "github:o/r?rev=sha1-AAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        "github owner=o repo=r rev=sha1-AAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        "github:o/r?rev=sha1-AAAAAAAAAAAAAAAAAAAAAAAAAAA%3D"
    )]
    #[case(
        "github:o/r?rev=00000000000000000000000000000000",
        "github owner=o repo=r rev=00000000000000000000000000000000",
        "github:o/r?rev=00000000000000000000000000000000"
    )]
    #[case(
        "github:NixOS/nixpkgs?narHash=x",
        "github owner=NixOS repo=nixpkgs",
        "github:NixOS/nixpkgs?narHash=x"
    )]
    #[case(
        "github:NixOS/nixpkgs?host=foo.com",
        "github owner=NixOS repo=nixpkgs",
        "github:NixOS/nixpkgs?host=foo.com"
    )]
    #[case(
        "github:NixOS/nixpkgs?dir=lib",
        "github owner=NixOS repo=nixpkgs",
        "github:NixOS/nixpkgs?dir=lib"
    )]
    #[case("/", "path /", "path:/")]
    #[case("path:", "path /", "path:/")]
    #[case("path:./foo", "path ./foo", "path:./foo")]
    #[case("/foo?dir=lib", "path /foo", "path:/foo?dir=lib")]
    #[case("/a b", "path /a b", "path:/a b")]
    #[case("/a[b", "path /a[b", "path:/a[b")]
    #[case("file:.", "file file .", "file://.")]
    #[case(
        "git+https://h/r?rev=abc",
        "git https h/r rev=abc",
        "git+https://h/r?rev=abc"
    )]
    #[case(
        "git+https://h.org/r?submodules=true",
        "git https h.org/r submodules=false",
        "git+https://h.org/r?submodules=0"
    )]
    #[case(
        "git+https://h/r?ref=_under",
        "git https h/r ref=_under",
        "git+https://h/r?ref=_under"
    )]
    #[case(
        "git+https://h/r?ref=x,y",
        "git https h/r ref=x,y",
        "git+https://h/r?ref=x%2Cy"
    )]
    #[case(
        "NixOS/nixpkgs",
        "indirect id=NixOS ref=nixpkgs",
        "flake:NixOS/nixpkgs"
    )]
    #[case(
        "nixpkgs/nixos-25.05",
        "indirect id=nixpkgs ref=nixos-25.05",
        "flake:nixpkgs/nixos-25.05"
    )]
    #[case(
        "flake:nixpkgs?ref=a..b",
        "indirect id=nixpkgs ref=a..b",
        "flake:nixpkgs?ref=a..b"
    )]
    fn accepted_everywhere(#[case] input: &str, #[case] fields: &str, #[case] display: &str) {
        for context in [
            Context::Standalone,
            Context::FlakeInput,
            Context::Installable,
        ] {
            accepts(input, context, fields, display);
        }
    }

    /// Rejected the same way in every context.
    #[rstest]
    #[case("github:NixOS/nixpkgs?rev=abc", Rejected::InvalidRev)]
    #[case(
        "github:o/r?rev=0000000000000000000000000000000000000000000000000000000000000000",
        Rejected::InvalidRev
    )]
    #[case("nixpkgs/a..b", Rejected::InvalidRef)]
    #[case("a/b/c/d", Rejected::Malformed)]
    #[case("foo:bar", Rejected::UriType)]
    #[case("path:/foo?unknown=1", Rejected::Param)]
    #[case("path:/foo?ref=main", Rejected::Param)]
    #[case("path:/foo?revCount=x", Rejected::Malformed)]
    #[case("/foo?unknown=1", Rejected::Param)]
    // A url that fails as a url falls back to a path, which then rejects
    // the query. The url-level error is the one reported.
    #[case("git+https://e.org/r?ref=a..b", Rejected::InvalidRef)]
    #[case("git+https://h/r?ref=a.lock", Rejected::InvalidRef)]
    #[case("github:NixOS/nixpkgs?unknown=1", Rejected::Param)]
    #[case("github:NixOS/nixpkgs?tag=v1", Rejected::Param)]
    #[case("github:NixOS/nixpkgs?submodules=1", Rejected::Param)]
    #[case("github:o/r?treeHash=abc", Rejected::Param)]
    #[case("gitlab:o/r?unknown=1", Rejected::Param)]
    #[case("sourcehut:~o/r?unknown=1", Rejected::Param)]
    #[case("github:o/r/main?ref=x", Rejected::DuplicateRef)]
    fn rejected_everywhere(#[case] input: &str, #[case] kind: Rejected) {
        for context in [
            Context::Standalone,
            Context::FlakeInput,
            Context::Installable,
        ] {
            rejects(input, context, kind);
        }
    }

    /// `builtins.parseFlakeRef` rejects relative paths, so a string that
    /// is neither a flake id nor a valid url is an error there. A flake
    /// input keeps it as a relative path instead.
    #[rstest]
    #[case("./foo", Rejected::RelativePath, "path ./foo", "path:./foo")]
    #[case(".", Rejected::RelativePath, "path .", "path:.")]
    #[case(
        "nixpkgs?dir=lib",
        Rejected::RelativePath,
        "path nixpkgs",
        "path:nixpkgs?dir=lib"
    )]
    #[case(
        "nixpkgs/.x",
        Rejected::RelativePath,
        "path nixpkgs/.x",
        "path:nixpkgs/.x"
    )]
    #[case(
        "nixpkgs//main",
        Rejected::RelativePath,
        "path nixpkgs//main",
        "path:nixpkgs//main"
    )]
    #[case("foo bar", Rejected::RelativePath, "path foo bar", "path:foo bar")]
    #[case(":", Rejected::RelativePath, "path :", "path::")]
    #[case(
        "git@github.com:NixOS/nixpkgs",
        Rejected::ScpStyleUrl,
        "path git@github.com:NixOS/nixpkgs",
        "path:git@github.com:NixOS/nixpkgs"
    )]
    #[case(
        "git+ssh://git@github.com:NixOS/nixpkgs",
        Rejected::ScpStyleUrl,
        "path git+ssh://git@github.com:NixOS/nixpkgs",
        "path:git+ssh://git@github.com:NixOS/nixpkgs"
    )]
    #[case(
        "github:NixOS/nixpkgs/a..b",
        Rejected::InvalidRef,
        "path github:NixOS/nixpkgs/a..b",
        "path:github:NixOS/nixpkgs/a..b"
    )]
    #[case(
        "github:NixOS/nixpkgs?lastModified=1",
        Rejected::Param,
        "path github:NixOS/nixpkgs",
        "path:github:NixOS/nixpkgs?lastModified=1"
    )]
    #[case("github:o", Rejected::Malformed, "path github:o", "path:github:o")]
    #[case("github:", Rejected::Malformed, "path github:", "path:github:")]
    #[case(
        "flake:nixpkgs/main/0000000000000000000000000000000000000000000000000000000000000000",
        Rejected::InvalidRev,
        "path flake:nixpkgs/main/0000000000000000000000000000000000000000000000000000000000000000",
        "path:flake:nixpkgs/main/0000000000000000000000000000000000000000000000000000000000000000"
    )]
    fn relative_path_fallback(
        #[case] input: &str,
        #[case] standalone: Rejected,
        #[case] fields: &str,
        #[case] display: &str,
    ) {
        rejects(input, Context::Standalone, standalone);
        accepts(input, Context::FlakeInput, fields, display);
        accepts(input, Context::Installable, fields, display);
    }

    /// Only installables take a `#fragment`.
    #[rstest]
    #[case(
        "github:NixOS/nixpkgs#hello",
        "github owner=NixOS repo=nixpkgs #hello",
        "github:NixOS/nixpkgs#hello"
    )]
    #[case("nixpkgs#hello", "indirect id=nixpkgs #hello", "flake:nixpkgs#hello")]
    #[case("/foo#bar", "path /foo #bar", "path:/foo#bar")]
    #[case(
        "github:o/r?dir=lib#x.y",
        "github owner=o repo=r #x.y",
        "github:o/r?dir=lib#x.y"
    )]
    fn fragment_only_in_installables(
        #[case] input: &str,
        #[case] fields: &str,
        #[case] display: &str,
    ) {
        rejects(input, Context::Standalone, Rejected::Fragment);
        rejects(input, Context::FlakeInput, Rejected::Fragment);
        accepts(input, Context::Installable, fields, display);
    }

    #[test]
    fn relative_path_fragment_in_installable() {
        rejects("./foo#bar", Context::Standalone, Rejected::RelativePath);
        rejects("./foo#bar", Context::FlakeInput, Rejected::Fragment);
        accepts(
            "./foo#bar",
            Context::Installable,
            "path ./foo #bar",
            "path:./foo#bar",
        );
    }

    #[test]
    fn empty_fragment_is_not_a_fragment() {
        // Only a non-empty fragment is rejected.
        accepts(
            "github:o/r#",
            Context::Standalone,
            "github owner=o repo=r #",
            "github:o/r#",
        );
    }

    #[test]
    fn three_segment_indirect_takes_sha1_rev() {
        let input = format!("nixpkgs/main/{REV}");
        accepts(
            &input,
            Context::Standalone,
            &format!("indirect id=nixpkgs ref=main rev={REV}"),
            &format!("flake:nixpkgs/main/{REV}"),
        );
    }
}
