use winnow::{
    ModalResult, Parser,
    combinator::{alt, opt, repeat, separated_pair},
    error::{StrContext, StrContextValue},
    token::{any, rest, take_till, take_until},
};

use crate::{
    error::{NixUriError, NixUriResult, run_partial},
    flakeref::{
        FlakeRef, FlakeRefType, GitForge, LocationParamKeys, LocationParameters, RefLocation,
        TransportLayer, encoding,
        location_params::ParamRefRev,
        validators::{looks_like_rev, parse_bool_param, validated_host_name, validated_ref_name},
    },
};

/// Raw `(key, value)` pairs as they appear in a query string, before the
/// routing pass in [`route_location_params`] turns them into typed slots
/// on [`LocationParameters`] / [`ParamRefRev`]. Keys may carry a leading
/// `&` left over from the param parser; the `LocationParamKeys` `FromStr`
/// strips it.
pub(crate) type RawParamValues<'i> = Vec<(&'i str, &'i str)>;

// TODO: use a param-specific parser, handle the inversion specifically.
/// Take all that is behind the "?" tag.
/// Returns everything prior as not parsed.
///
/// `ref` / `rev` query keys are pulled out into a [`ParamRefRev`] side-channel
/// rather than stashed inside [`LocationParameters`], because the typed
/// destination for those values lives on the `FlakeRef`'s kind. The caller
/// applies them via [`FlakeRefType::set_ref`] / [`FlakeRefType::set_rev`] and
/// flips the kind's [`RefLocation`] to `QueryParameter`.
pub(crate) fn parse_params<'i>(
    input: &mut &'i str,
) -> ModalResult<(&'i str, Option<RawParamValues<'i>>)> {
    // Routing (and bool-parse validation that surfaces
    // `NixUriError::InvalidValue`) happens in `route_location_params`
    // because winnow's `ModalResult` cannot carry a `NixUriError`.
    let maybe_flake_type = opt(take_until(0.., "?")).parse_next(input)?;

    if let Some(flake_type) = maybe_flake_type {
        // Discard leading "?".
        let _q = any.parse_next(input)?;
        let param_values: Vec<(&str, &str)> = repeat(
            0..,
            separated_pair(
                take_until(0.., "="),
                '='.context(StrContext::Expected(StrContextValue::CharLiteral('='))),
                alt((take_until(0.., "&"), take_till(0.., |c| c == '#'))),
            ),
        )
        .context(StrContext::Label("param_fetch"))
        .parse_next(input)?;
        Ok((flake_type, Some(param_values)))
    } else {
        // No "?": the entire input is the prefix.
        let prefix = rest.parse_next(input)?;
        Ok((prefix, None))
    }
}

/// Route raw `(key, value)` pairs from [`parse_params`] into the typed
/// destinations: typed slots on [`LocationParameters`], typed bool
/// conversion for the seven Git-flavoured params, and the `ref` / `rev`
/// side-channel ([`ParamRefRev`]). Keys without a typed slot
/// (e.g. `name`, `treeHash`) round-trip through
/// `LocationParameters::arbitrary`. Every scheme accepts every key:
/// the parser mirrors Nix's permissive `parseFlakeRef` instead of
/// gating per scheme.
///
/// Surfaces [`NixUriError::InvalidValue`] when a typed boolean param
/// (`lfs`, `exportIgnore`, `allRefs`, `verifyCommit`, `submodules`,
/// `shallow`) carries a value outside Nix's URL-time coercion, which
/// is strictly `value == "1"` (i.e. `"1"` or `"0"`; `"true"` /
/// `"false"` are rejected so the parse-time diagnostic is preserved).
pub(crate) fn route_location_params(
    values: RawParamValues<'_>,
) -> Result<(LocationParameters, ParamRefRev), NixUriError> {
    let mut params = LocationParameters::default();
    let mut ref_rev = ParamRefRev::default();
    for (param, value) in values {
        if let Ok(key) = param.parse::<LocationParamKeys>() {
            // Match Nix's query decoding: every value is percent-decoded
            // before it lands in a typed slot or the arbitrary bag, so
            // the in-memory representation is the raw user string and
            // Display re-encodes uniformly.
            let decoded = encoding::decode_percent(value)?.into_owned();
            match key {
                LocationParamKeys::Dir => params.set_dir(Some(decoded)),
                LocationParamKeys::NarHash => params.set_nar_hash(Some(decoded)),
                LocationParamKeys::LastModified => {
                    params.set_last_modified(Some(decoded));
                }
                LocationParamKeys::RevCount => params.set_rev_count(Some(decoded)),
                LocationParamKeys::Host => {
                    params.set_host(Some(validated_host_name(&decoded)?));
                }
                LocationParamKeys::Ref => ref_rev.r#ref = Some(decoded),
                LocationParamKeys::Rev => ref_rev.rev = Some(decoded),
                LocationParamKeys::Submodules => {
                    params.set_submodules(Some(parse_bool_param("submodules", &decoded)?));
                }
                LocationParamKeys::Shallow => {
                    params.set_shallow(Some(parse_bool_param("shallow", &decoded)?));
                }
                LocationParamKeys::Lfs => {
                    params.set_lfs(Some(parse_bool_param("lfs", &decoded)?));
                }
                LocationParamKeys::ExportIgnore => {
                    params.set_export_ignore(Some(parse_bool_param("exportIgnore", &decoded)?));
                }
                LocationParamKeys::AllRefs => {
                    params.set_all_refs(Some(parse_bool_param("allRefs", &decoded)?));
                }
                LocationParamKeys::VerifyCommit => {
                    params.set_verify_commit(Some(parse_bool_param("verifyCommit", &decoded)?));
                }
                LocationParamKeys::Keytype => params.set_keytype(Some(decoded)),
                LocationParamKeys::PublicKey => params.set_public_key(Some(decoded)),
                LocationParamKeys::PublicKeys => params.set_public_keys(Some(decoded)),
                LocationParamKeys::Arbitrary(k) => {
                    params.add_arbitrary((k, decoded));
                }
            }
        }
    }
    Ok((params, ref_rev))
}

/// Apply a [`ParamRefRev`] to a [`FlakeRef`]: write the ref/rev into the
/// kind's typed slots and mark the kind's [`RefLocation`] as
/// `QueryParameter` so a Display round-trip preserves the `?ref=` / `?rev=`
/// form. No-op if both slots are empty.
///
/// Validates `?rev=` against [`looks_like_rev`]: the path-component
/// side classifies a 40-hex (SHA-1) or 64-hex (SHA-256) value into `rev`
/// and anything else into `ref_`, but the query side has no such
/// classifier and would otherwise accept `?rev=main` verbatim. Returns
/// [`NixUriError::InvalidValue`] `{ field: "rev", .. }` for values that
/// are not a 40- or 64-character hex string.
///
/// Validates `?ref=` against
/// [`validate_ref_name`](crate::flakeref::validators::validate_ref_name),
/// matching Nix's accepted ref-name shape. Returns
/// [`NixUriError::InvalidValue`] `{ field: "ref", .. }` for values that
/// would not survive the downstream fetcher's own ref check.
///
/// Only writes a slot when the incoming value is `Some`. Preserving the
/// existing path-component value when the matching query key is absent is
/// what lets the post-parse mutual-exclusion check (see
/// [`validate_gitforge_ref_rev_exclusion`]) observe both sources.
///
/// `Path` has no typed `ref_` slot (Nix's path scheme has no place for
/// one), so a `?ref=` query value reaches the kind's no-op `set_ref` and
/// is dropped. This mirrors Nix's permissive parse and trades round-trip
/// fidelity on that one shape against accepting every input Nix accepts.
pub(crate) fn apply_param_ref_rev(
    flake_ref: &mut FlakeRef,
    ref_rev: ParamRefRev,
) -> Result<(), NixUriError> {
    if ref_rev.r#ref.is_none() && ref_rev.rev.is_none() {
        return Ok(());
    }
    if let Some(rev) = ref_rev.rev.as_deref() {
        if !looks_like_rev(rev) {
            return Err(NixUriError::InvalidValue {
                field: "rev",
                reason: "expected 40-hex (SHA-1) or 64-hex (SHA-256) commit".to_string(),
            });
        }
    }
    if let Some(value) = ref_rev.r#ref.as_deref() {
        let validated = validated_ref_name(value)?;
        flake_ref.kind_mut().set_ref(Some(validated));
    }
    if ref_rev.rev.is_some() {
        flake_ref.kind_mut().set_rev(ref_rev.rev);
    }
    flake_ref
        .kind_mut()
        .set_ref_location(RefLocation::QueryParameter);
    Ok(())
}

/// Reject a [`FlakeRefType::GitForge`] kind whose `ref_` and `rev` slots
/// are both populated. Matches Nix's behaviour: a git-forge URL with both
/// fields set is rejected.
///
/// Surfaces as [`NixUriError::FieldConflict`] (a structural relationship
/// between two fields) rather than the value-shape
/// [`NixUriError::InvalidValue`]; pattern-matching consumers can therefore
/// distinguish "user typo'd a field" from "user combined two mutually
/// exclusive fields".
///
/// Indirect's canonical `flake:id/ref/rev` form and Resource(Git)'s
/// `?ref=branch&rev=hex` are both legitimate and stay untouched.
fn validate_gitforge_ref_rev_exclusion(flake_ref: &FlakeRef) -> Result<(), NixUriError> {
    if let FlakeRefType::GitForge(GitForge {
        ref_: Some(_),
        rev: Some(_),
        ..
    }) = flake_ref.kind()
    {
        return Err(NixUriError::FieldConflict {
            left: "ref",
            right: "rev",
        });
    }
    Ok(())
}

/// Detect the SCP-style Git URL shape `[<user>@]<host>:<path>` and return
/// the canonical `git+ssh://[<user>@]<host>/<path>` rewrite.
///
/// SCP form is the default output of `git remote add` / `git clone`, but the
/// bare colon between host and path makes generic URL parsers (and ours)
/// choke on the scheme dispatch. Nix canonicalises this to an
/// `ssh://`-transport URL before dispatch; nix-uri does the same, but
/// expressed in the crate's own grammar where `Resource(Git, Ssh)` Displays
/// as `git+ssh://...`. The SCP form is one-directional: parse accepts it,
/// `Display` always emits the canonical form.
///
/// Accepts the three shapes Nix's SCP detector recognises beyond the
/// basic `git@host:repo`:
///
/// - **No userinfo.** `github.com:nixos/nixpkgs` rewrites to
///   `git+ssh://github.com/nixos/nixpkgs`. The userinfo `@`-prefix is
///   optional; when absent the host is everything up to the colon.
///   Disambiguated from nix-uri's own bare-colon schemes (`github:`,
///   `flake:`, `path:`, etc.) by requiring the host segment to look
///   host-like (at least one `.` or a bracketed IPv6 literal) when no
///   `@` is present.
/// - **Absolute path after the colon.** `git@host:/srv/git/repo.git`
///   rewrites to `git+ssh://git@host/srv/git/repo.git`. The leading `/`
///   in the SCP path is preserved as the URL path (one slash, not two:
///   the `git+ssh://<authority>` already terminates with the slot
///   immediately before the path).
/// - **IPv6 host.** `user@[::1]:repo` rewrites to
///   `git+ssh://user@[::1]/repo`. Bracket parsing prefers `@[` if
///   present, else a leading `[`; the `:` SCP separator must be the
///   byte immediately after the closing `]` (otherwise we bail out and
///   let the input fall through to the regular dispatch). Zone-ids
///   inside the brackets (e.g. `[fe80::1%25eth0]`) round-trip
///   verbatim; we do not crack the IPv6 literal open.
///
/// Returns `None` for any input that is not unambiguously SCP-shaped:
///
/// - input contains `://` (already a URL with an authority);
/// - no `:` separator at all, or a `/` appears before the first `:`
///   (making it a local path, not SCP);
/// - the host segment is empty, or (when no `@` is present) looks
///   like a scheme prefix rather than a hostname;
/// - bracketed host with a malformed `[...]:` boundary;
/// - empty path after the `:`.
pub(crate) fn parse_scp_style(input: &str) -> Option<String> {
    if input.contains("://") {
        return None;
    }

    // Locate the SCP `:` separator and the host substring that precedes it.
    // IPv6-bracket detection runs first because a bracketed host contains
    // colons of its own; the SCP `:` lives immediately after `]`.
    let bracket_start = input
        .find("@[")
        .map(|p| p + 1)
        .or_else(|| input.starts_with('[').then_some(0));

    let (host_with_userinfo, path) = if let Some(open) = bracket_start {
        let close_rel = input[open + 1..].find(']')?;
        let close = open + 1 + close_rel;
        // Bail when a `:` exists after `]` but is not the very next
        // byte; that keeps the rewrite path narrow and avoids silently
        // dropping characters.
        let after_bracket = &input[close + 1..];
        if !after_bracket.starts_with(':') {
            return None;
        }
        (&input[..=close], &after_bracket[1..])
    } else {
        let colon = input.find(':')?;
        let host_part = &input[..colon];
        // A `/` before the first `:` means a local path (e.g. `foo/bar:baz`),
        // not SCP. Matches git's local-vs-ssh disambiguation.
        if host_part.contains('/') {
            return None;
        }
        (host_part, &input[colon + 1..])
    };

    // Split optional userinfo `<user>@` off the host segment.
    let (userinfo, host) = match host_with_userinfo.rfind('@') {
        Some(at) => {
            let user = &host_with_userinfo[..at];
            // Userinfo must be non-empty when `@` is present, must not
            // contain `:` (which would make it a different kind of
            // authority), and the host after `@` must be non-empty.
            if user.is_empty() || user.contains(':') {
                return None;
            }
            (Some(user), &host_with_userinfo[at + 1..])
        }
        None => (None, host_with_userinfo),
    };
    if host.is_empty() {
        return None;
    }

    // Without a userinfo segment the host portion is ambiguous with
    // nix-uri's own bare-colon scheme prefixes (`github:`, `flake:`,
    // `path:`, ...). Require it to look host-like (a `.` somewhere, or a
    // bracketed IPv6 literal) to avoid stealing those inputs from the
    // scheme dispatch. Hosts with a userinfo segment are not subject to
    // this filter because `<user>@<host>` already disambiguates.
    if userinfo.is_none() && !host.contains('.') && !host.starts_with('[') {
        return None;
    }

    if path.is_empty() {
        return None;
    }

    // Preserve a leading `/` rather than doubling it: the URL authority
    // already terminates immediately before the path slot, so an absolute
    // SCP path `host:/abs/p` becomes `git+ssh://host/abs/p` (one slash),
    // matching the canonical `ssh://host/abs/p` shape.
    let path_separator = if path.starts_with('/') { "" } else { "/" };
    match userinfo {
        Some(user) => Some(format!("git+ssh://{user}@{host}{path_separator}{path}")),
        None => Some(format!("git+ssh://{host}{path_separator}{path}")),
    }
}

pub(crate) fn parse_nix_uri(input: &str) -> NixUriResult<FlakeRef> {
    // Basic sanity checks.
    if input.trim().is_empty()
        || (input.trim() == "/")
        || (input.trim() == ":")
        || (input.trim() == "?")
        || (!input.is_ascii())
        || (!input.chars().all(|c| !c.is_control()))
        || (!input.chars().all(|c| !c.is_ascii_control()))
        || (input.ends_with(char::is_whitespace))
        || (input.starts_with(char::is_whitespace))
    {
        return Err(NixUriError::InvalidUrl(input.into()));
    }

    let rewritten = parse_scp_style(input);
    let input = rewritten.as_deref().unwrap_or(input);

    // Slice off the trailing `#fragment` before handing the rest to the type
    // and param parsers; both treat `#` as a terminator already, so removing
    // it here just avoids leaving the unparsed remainder on the floor. The
    // raw slice is percent-decoded so the in-memory fragment is the
    // user-visible value and Display re-encodes uniformly.
    let (head, fragment) = match input.find('#') {
        Some(pos) => (
            &input[..pos],
            Some(encoding::decode_percent(&input[pos + 1..])?.into_owned()),
        ),
        None => (input, None),
    };

    let (_, (type_prefix, raw_values)) = run_partial(input, head, parse_params)?;
    let mut flake_ref = FlakeRef::default().with_kind(FlakeRefType::parse_type(type_prefix)?);
    if let Some(values) = raw_values {
        let (params, ref_rev) = route_location_params(values)?;
        flake_ref.replace_params(params);
        apply_param_ref_rev(&mut flake_ref, ref_rev)?;
    }
    validate_gitforge_ref_rev_exclusion(&flake_ref)?;
    flake_ref.set_fragment(fragment);

    Ok(flake_ref)
}

/// Parses the part AFTER the leading `+` in `<scheme>+<layer>`. For input
/// `"git+fizzbuzz"` returns `"fizzbuzz"`. The `+` and the prefix are both
/// consumed.
pub(crate) fn parse_from_transport_type<'i>(input: &mut &'i str) -> ModalResult<&'i str> {
    let _prefix = take_until(0.., "+").parse_next(input)?;
    let _plus = any.parse_next(input)?;
    let layer = rest.parse_next(input)?;
    Ok(layer)
}

pub(crate) fn is_tarball(input: &str) -> bool {
    let valid_extensions = &[
        ".zip", ".tar", ".tgz", ".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst",
    ];
    valid_extensions.iter().any(|&ext| input.ends_with(ext))
}

#[allow(unused)]
pub(crate) fn is_file(input: &str) -> bool {
    !is_tarball(input)
}

// Parse the transport type itself.
pub(crate) fn parse_transport_type(input: &str) -> Result<TransportLayer, NixUriError> {
    let (_, layer_str) = run_partial(input, input, parse_from_transport_type)?;
    TryInto::<TransportLayer>::try_into(layer_str)
}

#[allow(dead_code)]
pub(crate) fn parse_sep(input: &mut &str) -> ModalResult<(char, char, char)> {
    (
        ':'.context(StrContext::Expected(StrContextValue::CharLiteral(':'))),
        '/'.context(StrContext::Expected(StrContextValue::CharLiteral('/'))),
        '/'.context(StrContext::Expected(StrContextValue::CharLiteral('/'))),
    )
        .context(StrContext::Label("location separator"))
        .parse_next(input)
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
        assert!(is_file(filename));
    }
    #[test]
    fn check_file() {
        let filename = "example";
        assert!(is_file(filename));
    }
}

#[cfg(test)]
mod ref_name_validation {
    //! Public-surface coverage for parse-time `ref` validation. Nix's
    //! ref-name shape is enforced at every routing site that hands a
    //! string into a kind's typed `ref_` slot; these tests pin the
    //! user-visible behaviour of the rejection paths.
    use crate::{FlakeRef, NixUriError};
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
        // GitHub path-component ref: `github:o/r/.hidden`.
        assert_matches!(
            "github:o/r/.hidden".parse::<FlakeRef>(),
            Err(NixUriError::InvalidValue { field: "ref", .. })
        );
    }

    #[test]
    fn indirect_ref_rejects_leading_dash() {
        assert_matches!(
            "flake:nixpkgs/-bad".parse::<FlakeRef>(),
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
    //! Public-surface coverage for parse-time `?rev=` validation. Nix
    //! accepts SHA-1 (40 hex) and SHA-256 (64 hex) commit hashes for
    //! git inputs; anything else surfaces as
    //! `InvalidValue { field: "rev", .. }`.
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
    fn rev_query_64_hex_accepted() {
        let parsed: FlakeRef =
            "github:o/r?rev=0000000000000000000000000000000000000000000000000000000000000000"
                .parse()
                .expect("64-hex rev (SHA-256) must parse");
        assert_eq!(
            parsed.to_string(),
            "github:o/r?rev=0000000000000000000000000000000000000000000000000000000000000000"
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
    //! Public-surface coverage for the SCP-style Git URL shape
    //! (`<user>@<host>:<path>`). Nix canonicalises this to an
    //! ssh-transport form before dispatch; in nix-uri's grammar that
    //! canonical form is `git+ssh://<user>@<host>/<path>` (a
    //! `Resource(Git, Ssh)`). The rewrite happens at parser entry, so
    //! `Display` always emits the canonical form rather than the SCP
    //! pre-image.
    use crate::{FlakeRef, FlakeRefType, ResourceType, TransportLayer};
    use rstest::rstest;

    #[rstest]
    #[case::github(
        "git@github.com:nixos/nixpkgs",
        "git+ssh://git@github.com/nixos/nixpkgs"
    )]
    #[case::gitlab("git@gitlab.com:owner/repo", "git+ssh://git@gitlab.com/owner/repo")]
    #[case::sourcehut("git@git.sr.ht:~user/proj", "git+ssh://git@git.sr.ht/~user/proj")]
    #[case::self_hosted(
        "git@self-hosted.example.com:team/svc",
        "git+ssh://git@self-hosted.example.com/team/svc"
    )]
    fn git_scp_form_for_each_forge(#[case] scp: &str, #[case] canonical: &str) {
        let parsed: FlakeRef = scp.parse().expect("SCP form must parse");
        match parsed.kind() {
            FlakeRefType::Resource(u) => {
                assert_eq!(u.res_type, ResourceType::Git, "for {scp}");
                assert_eq!(u.transport_type, Some(TransportLayer::Ssh), "for {scp}");
            }
            other => panic!("expected Resource(Git, Ssh), got {other:?}"),
        }
        assert_eq!(parsed.to_string(), canonical);
        let reparsed: FlakeRef = canonical.parse().expect("canonical form must parse");
        assert_eq!(parsed, reparsed);
        assert_eq!(reparsed.to_string(), canonical);
    }

    #[rstest]
    #[case::trailing_slash("git@github.com:owner/repo/", "git+ssh://git@github.com/owner/repo/")]
    #[case::tilde_path("git@host.example:~svc/repo", "git+ssh://git@host.example/~svc/repo")]
    #[case::deep_path("git@host.example:a/b/c/d", "git+ssh://git@host.example/a/b/c/d")]
    #[case::dotted_user(
        "first.last@host.example:team/svc",
        "git+ssh://first.last@host.example/team/svc"
    )]
    fn git_scp_form_edge_cases(#[case] scp: &str, #[case] canonical: &str) {
        let parsed: FlakeRef = scp.parse().expect("SCP form must parse");
        assert_eq!(parsed.to_string(), canonical);
        let reparsed: FlakeRef = parsed.to_string().parse().unwrap();
        assert_eq!(parsed, reparsed);
    }

    #[test]
    fn explicit_git_ssh_scheme_unchanged() {
        let uri = "git+ssh://git@github.com/nixos/nixpkgs";
        let parsed: FlakeRef = uri.parse().unwrap();
        assert_eq!(parsed.to_string(), uri);
        match parsed.kind() {
            FlakeRefType::Resource(u) => {
                assert_eq!(u.transport_type, Some(TransportLayer::Ssh));
            }
            other => panic!("expected Resource(Git, Ssh), got {other:?}"),
        }
    }

    #[rstest]
    #[case::email_no_path("user@host")]
    #[case::empty_path_after_colon("git@host:")]
    #[case::empty_path_no_userinfo("github.com:")]
    fn scp_lookalikes_remain_rejected(#[case] uri: &str) {
        // The detector's narrowness is what stops these strings from being
        // silently rewritten into a parsing `git+ssh://...`. If a future
        // change loosened any predicate, the rewrite would succeed and the
        // input would start parsing instead of erroring; pin the error
        // shape at the public surface so that regression is caught.
        assert!(
            uri.parse::<FlakeRef>().is_err(),
            "{uri} must not be coerced into a valid FlakeRef",
        );
    }

    /// Coverage for the three SCP shapes Nix's detector accepts:
    ///   (a) no userinfo (`github.com:nixos/nixpkgs`),
    ///   (b) absolute path after the colon (`git@host:/abs/path`),
    ///   (c) IPv6 host (`user@[::1]:path`, with optional zone-id).
    ///
    /// The canonical-form column matches what `Display` re-emits after the
    /// rewrite to `git+ssh://...`; the per-case round-trip through `parse ->
    /// Display -> parse` pins that the widened detector did not also widen
    /// the *output* shape.
    #[rstest]
    #[case::no_userinfo("github.com:nixos/nixpkgs", "git+ssh://github.com/nixos/nixpkgs")]
    #[case::no_userinfo_with_subpath(
        "git.example.org:team/repo/sub",
        "git+ssh://git.example.org/team/repo/sub"
    )]
    #[case::absolute_path(
        "git@host.example:/srv/git/repo.git",
        "git+ssh://git@host.example/srv/git/repo.git"
    )]
    #[case::absolute_path_no_userinfo("host.example:/abs/path", "git+ssh://host.example/abs/path")]
    #[case::ipv6_host("user@[::1]:repo.git", "git+ssh://user@[::1]/repo.git")]
    #[case::ipv6_host_no_userinfo("[::1]:repo.git", "git+ssh://[::1]/repo.git")]
    #[case::ipv6_host_with_zone_id(
        "user@[fe80::1%25eth0]:repo",
        "git+ssh://user@[fe80::1%25eth0]/repo"
    )]
    fn scp_widened_shapes(#[case] scp: &str, #[case] canonical: &str) {
        let parsed: FlakeRef = scp.parse().expect("widened SCP form must parse");
        match parsed.kind() {
            FlakeRefType::Resource(u) => {
                assert_eq!(u.res_type, ResourceType::Git, "for {scp}");
                assert_eq!(u.transport_type, Some(TransportLayer::Ssh), "for {scp}");
            }
            other => panic!("expected Resource(Git, Ssh), got {other:?}"),
        }
        assert_eq!(parsed.to_string(), canonical, "Display canonical for {scp}");
        let reparsed: FlakeRef = canonical.parse().expect("canonical form must parse");
        assert_eq!(parsed, reparsed, "round-trip equality for {scp}");
        assert_eq!(
            reparsed.to_string(),
            canonical,
            "round-trip Display for {scp}"
        );
    }
}

#[cfg(test)]
mod arbitrary_query_keys {
    //! Public-surface coverage for the permissive-key contract. Every
    //! scheme accepts every query key: keys without a typed slot land
    //! verbatim in [`crate::LocationParameters`]'s arbitrary bag, mirroring
    //! Nix's `parseFlakeRef`. One representative case per scheme cluster
    //! pins that contract.
    use crate::FlakeRef;
    use rstest::rstest;

    fn assert_value_preserved(input: &str, key: &str, value: &str) {
        let parsed: FlakeRef = input
            .parse()
            .unwrap_or_else(|e| panic!("{input}: expected success, got {e:?}"));
        let rendered = parsed.to_string();
        let needle = format!("{key}={value}");
        assert!(
            rendered.contains(&needle),
            "{input}: `{needle}` missing in Display output `{rendered}`",
        );
    }

    #[rstest]
    #[case::github("github:o/r?treeHash=abc", "treeHash", "abc")]
    #[case::github_submodules("github:o/r?submodules=1", "submodules", "1")]
    #[case::gitlab("gitlab:o/r?wurzel=pfropf", "wurzel", "pfropf")]
    #[case::sourcehut("sourcehut:~o/r?treeHash=abc", "treeHash", "abc")]
    #[case::indirect("flake:nixpkgs?host=foo", "host", "foo")]
    #[case::git("git+ssh://example.com/repo?treeHash=abc", "treeHash", "abc")]
    #[case::mercurial("hg+https://example.com/repo?lfs=1", "lfs", "1")]
    #[case::path("path:/foo/bar?host=x", "host", "x")]
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
        // route into their typed slots regardless of scheme; keys with no
        // typed slot (`treeHash`, `wurzel`) land in the arbitrary bag.
        // Either way the parser does not reject, and Display preserves
        // the same `key=value` pair the user wrote.
        assert_value_preserved(input, key, value);
    }

    /// Universal locked-attrs set: Nix moves `narHash`, `lastModified`,
    /// and `revCount` out of the URL into `attrs` regardless of scheme.
    /// They are the keys a `flake.lock`'s `locked` block emits when
    /// round-trip serialised through Nix, so every scheme must accept
    /// them at parse time. One representative shape per scheme pins the
    /// contract.
    #[rstest]
    #[case::github("github:o/r?")]
    #[case::gitlab("gitlab:o/r?")]
    #[case::sourcehut("sourcehut:~o/r?")]
    #[case::indirect("flake:nixpkgs?")]
    #[case::git("git+ssh://example.com/repo?")]
    #[case::mercurial("hg+https://example.com/repo?")]
    #[case::path("path:/foo/bar?")]
    #[case::tarball("tarball+https://example.com/x.tar.gz?")]
    #[case::file("file+https://example.com/x?")]
    fn locked_attrs_accepted_on_every_scheme(#[case] prefix: &str) {
        let input = format!("{prefix}lastModified=1&narHash=sha256-abc&revCount=2");
        let parsed: FlakeRef = input
            .parse()
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
