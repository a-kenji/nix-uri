//! Recursive proptest generator + round-trip property for [`FlakeRef`].
//!
//! Bottom-up shape: small string strategies feed into the typed structs
//! ([`GitForge`], [`FlakeRefType::Indirect`], [`FlakeRefType::Path`],
//! [`ResourceUrl`]), which feed into [`FlakeRefType`], which feeds into
//! [`FlakeRef`] alongside the fragment and [`LocationParameters`] strategies.
//!
//! The generator is constrained to values whose `Display` round-trips
//! through `FromStr` cleanly. Where the parser/`Display` pair makes a
//! shape unrepresentable post-round-trip (e.g. a `GitForge` with both
//! `ref_` and `rev` in the path-component form, where `Display` drops one),
//! the generator avoids producing it. Each constraint is documented at
//! its point of use.

use proptest::prelude::*;

use super::{
    FlakeRef, FlakeRefType, GitForge, GitForgePlatform, LocationParameters, RefLocation,
    ResourceType, ResourceUrl, TransportLayer,
};

/// A ref-name string. Bounded length, no `/` or other URL-special characters,
/// and explicitly excludes 40-hex (SHA-1) and 64-hex (SHA-256) strings,
/// which `looks_like_rev` would classify as `rev`.
fn ref_string_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_.\\-]{0,15}".prop_filter(
        "ref must not be 40- or 64-hex (would classify as rev)",
        |s: &String| !super::validators::looks_like_rev(s),
    )
}

/// A revision string accepted by [`super::validators::looks_like_rev`]:
/// 40-hex (SHA-1) or 64-hex (SHA-256). Both lengths must round-trip, so
/// the generator covers each.
fn rev_string_strategy() -> impl Strategy<Value = String> {
    prop_oneof!["[0-9a-f]{40}", "[0-9a-f]{64}"]
}

fn ref_location_strategy() -> impl Strategy<Value = RefLocation> {
    prop_oneof![
        Just(RefLocation::PathComponent),
        Just(RefLocation::QueryParameter),
    ]
}

fn opt_ref_strategy() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), ref_string_strategy().prop_map(Some)]
}

fn opt_rev_strategy() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), rev_string_strategy().prop_map(Some)]
}

fn platform_strategy() -> impl Strategy<Value = GitForgePlatform> {
    prop_oneof![
        Just(GitForgePlatform::GitHub),
        Just(GitForgePlatform::GitLab),
        Just(GitForgePlatform::SourceHut),
    ]
}

fn owner_or_repo_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_-]{0,15}"
}

/// GitLab-only owner: a 1..=4 segment subgroup path. The single-segment
/// case keeps coverage of the plain owner shape; the multi-segment cases
/// exercise the encode-decode pair on `Display`/parse round-trip
/// (`encode_path_segment` re-emits decoded `/` as `%2F`).
fn gitlab_owner_strategy() -> impl Strategy<Value = String> {
    proptest::collection::vec("[a-zA-Z][a-zA-Z0-9_-]{0,7}", 1..=4).prop_map(|segs| segs.join("/"))
}

fn id_strategy() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_-]{0,15}"
}

/// `RefLocation` is a routing tag that names where a *present* value would be
/// rendered. When neither `ref_` nor `rev` is set, the tag is informational;
/// the parser always emits `PathComponent` in that case, so the generator
/// pins it the same way to keep equality.
fn normalise_location(
    ref_: Option<String>,
    rev: Option<String>,
    location: RefLocation,
) -> (Option<String>, Option<String>, RefLocation) {
    if ref_.is_none() && rev.is_none() {
        (ref_, rev, RefLocation::PathComponent)
    } else {
        (ref_, rev, location)
    }
}

fn git_forge_strategy() -> impl Strategy<Value = GitForge> {
    (
        platform_strategy(),
        owner_or_repo_strategy(),
        gitlab_owner_strategy(),
        owner_or_repo_strategy(),
        opt_ref_strategy(),
        opt_rev_strategy(),
        ref_location_strategy(),
    )
        .prop_map(
            |(platform, plain_owner, gitlab_owner, repo, ref_, rev, location)| {
                // GitLab admits subgroup-form (`/`-bearing) owners; GitHub
                // and SourceHut do not. Pick the platform-appropriate owner
                // so the round-trip property exercises the GitLab encode-
                // decode pair without producing strings the validator
                // refuses for the other forges.
                let owner = match platform {
                    GitForgePlatform::GitLab => gitlab_owner,
                    _ => plain_owner,
                };
                // GitForge rejects `ref` and `rev` together post-parse
                // (`parser::validate_gitforge_ref_rev_exclusion`), so a
                // generated value with both populated would Display to a
                // string the parser refuses, breaking the round-trip
                // property. Collapse to ref-only in that case; the
                // rev-only and ref-only branches exercise the surviving
                // shapes.
                let (ref_, rev) = if ref_.is_some() && rev.is_some() {
                    (ref_, None)
                } else {
                    (ref_, rev)
                };
                let (ref_, rev, location) = normalise_location(ref_, rev, location);
                GitForge {
                    platform,
                    owner,
                    repo,
                    ref_,
                    rev,
                    location,
                }
            },
        )
}

fn indirect_strategy() -> impl Strategy<Value = FlakeRefType> {
    (
        id_strategy(),
        opt_ref_strategy(),
        opt_rev_strategy(),
        ref_location_strategy(),
    )
        .prop_map(|(id, ref_, rev, location)| {
            let (ref_, rev, location) = normalise_location(ref_, rev, location);
            FlakeRefType::Indirect {
                id,
                ref_,
                rev,
                location,
            }
        })
}

/// Path content: avoid `[`, `]`, `?`, `#` (which the parser explicitly
/// rejects in `path:` `rest_input`). All ASCII, no controls. The absolute
/// form pins the first character after the leading `/` to a non-slash
/// so the body never collapses into `path://...`, the URL-authority
/// shape Nix rejects.
///
/// Each generated value carries an optional `?rev=<40hex>` pin so the
/// round-trip property exercises locked store-path inputs.
fn path_strategy() -> impl Strategy<Value = FlakeRefType> {
    let body = prop_oneof![
        "/[a-zA-Z0-9._-][a-zA-Z0-9._/-]{0,15}",
        r"\.\.?/[a-zA-Z0-9._/-]{1,16}",
        Just(".".to_string()),
        Just("..".to_string()),
    ];
    (body, opt_rev_strategy()).prop_map(|(path, rev)| FlakeRefType::Path { path, rev })
}

fn transport_strategy() -> impl Strategy<Value = TransportLayer> {
    prop_oneof![
        Just(TransportLayer::Http),
        Just(TransportLayer::Https),
        Just(TransportLayer::Ssh),
        Just(TransportLayer::File),
    ]
}

/// Resource location: a `host/path` shape. The host part avoids `?`/`#`/`@`/`:`
/// which would either short-circuit the parser or trip the SCP-style detector
/// at parser entry (see `crate::parser::parse_scp_style`); SCP form is a
/// one-way input canonicalisation and so is excluded from the round-trip
/// property by construction.
fn resource_location_strategy() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9.]{0,15}/[a-zA-Z0-9._\\-/]{1,32}"
}

/// Tarball location: any host/path that ends with a Nix-recognised tarball
/// extension so `parser::is_tarball` re-classifies it as `Tarball` after
/// `Display` strips the `tarball+` application prefix.
fn tarball_location_strategy() -> impl Strategy<Value = String> {
    (
        "[a-z][a-z0-9.]{0,15}/[a-zA-Z0-9._\\-/]{1,16}",
        prop_oneof![
            Just(".zip"),
            Just(".tar"),
            Just(".tgz"),
            Just(".tar.gz"),
            Just(".tar.xz"),
            Just(".tar.bz2"),
            Just(".tar.zst"),
        ],
    )
        .prop_map(|(base, ext)| format!("{base}{ext}"))
}

/// File location: any host/path that does NOT end with a tarball extension,
/// keeping `parser::is_tarball` from re-routing the auto-classified parse
/// back into `Tarball`.
fn file_location_strategy() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9.]{0,15}/[a-zA-Z0-9._\\-/]{1,32}"
        .prop_filter("must not collide with a tarball extension", |s: &String| {
            !crate::parser::is_tarball(s)
        })
}

/// Tarball/File transport: only `http`/`https` survive `Display`'s
/// prefix-strip canonicalisation, since the bare-URL re-parse path
/// (`FlakeRefType::parse_plain_url`) only matches those two schemes.
fn curl_transport_strategy() -> impl Strategy<Value = TransportLayer> {
    prop_oneof![Just(TransportLayer::Http), Just(TransportLayer::Https)]
}

fn make_resource(
    res_type: ResourceType,
    location: String,
    transport_type: Option<TransportLayer>,
    ref_: Option<String>,
    rev: Option<String>,
) -> FlakeRefType {
    // Resource has no path-component ref/rev form; the parser
    // unconditionally flips `ref_location` to `QueryParameter` whenever it
    // routes a value out of the query string. Match that.
    let ref_location = if ref_.is_some() || rev.is_some() {
        RefLocation::QueryParameter
    } else {
        RefLocation::PathComponent
    };
    FlakeRefType::Resource(ResourceUrl {
        res_type,
        location,
        transport_type,
        ref_,
        rev,
        ref_location,
    })
}

fn git_resource_strategy() -> impl Strategy<Value = FlakeRefType> {
    // Git is the only Resource scheme that round-trips with a transport-less
    // form (`git://...`); the Display branch keeps the `git` prefix.
    (
        resource_location_strategy(),
        prop_oneof![
            Just(None::<TransportLayer>),
            transport_strategy().prop_map(Some),
        ],
        opt_ref_strategy(),
        opt_rev_strategy(),
    )
        .prop_map(|(location, transport_type, ref_, rev)| {
            make_resource(ResourceType::Git, location, transport_type, ref_, rev)
        })
}

fn mercurial_resource_strategy() -> impl Strategy<Value = FlakeRefType> {
    // Mercurial is only spellable as `hg+<transport>://`; Display keeps the
    // `hg+` prefix so any TransportLayer round-trips.
    (
        resource_location_strategy(),
        transport_strategy(),
        opt_ref_strategy(),
        opt_rev_strategy(),
    )
        .prop_map(|(location, transport_type, ref_, rev)| {
            make_resource(
                ResourceType::Mercurial,
                location,
                Some(transport_type),
                ref_,
                rev,
            )
        })
}

fn tarball_resource_strategy() -> impl Strategy<Value = FlakeRefType> {
    // After Display strips `tarball+`, only the `<http|https>://...tar.ext`
    // shape survives the round-trip: the bare URL is auto-classified as
    // `Tarball` by `is_tarball(location)`. Other transports / extensions
    // would re-parse to `File` or fail to resolve.
    //
    // `ref_` is pinned to `None`: Nix's curl-based fetcher excludes
    // `ref`, so the per-scheme allow-list rejects `?ref=` on Tarball.
    // Display emitting a populated `ref_` would produce a string the
    // parser refuses, breaking round-trip.
    (
        tarball_location_strategy(),
        curl_transport_strategy(),
        opt_rev_strategy(),
    )
        .prop_map(|(location, transport, rev)| {
            make_resource(ResourceType::Tarball, location, Some(transport), None, rev)
        })
}

fn file_resource_strategy() -> impl Strategy<Value = FlakeRefType> {
    // Mirror of `tarball_resource_strategy`. The location must NOT end in a
    // tarball extension, otherwise auto-classification would route the bare
    // URL into `Tarball` after Display. `ref_` is `None` for the same
    // reason as Tarball: Nix's curl-based fetcher excludes `ref`.
    (
        file_location_strategy(),
        curl_transport_strategy(),
        opt_rev_strategy(),
    )
        .prop_map(|(location, transport, rev)| {
            make_resource(ResourceType::File, location, Some(transport), None, rev)
        })
}

fn resource_strategy() -> impl Strategy<Value = FlakeRefType> {
    prop_oneof![
        git_resource_strategy(),
        mercurial_resource_strategy(),
        tarball_resource_strategy(),
        file_resource_strategy(),
    ]
}

fn kind_strategy() -> impl Strategy<Value = FlakeRefType> {
    prop_oneof![
        git_forge_strategy().prop_map(FlakeRefType::GitForge),
        indirect_strategy(),
        path_strategy(),
        resource_strategy(),
    ]
}

/// Free-form value strategy that occasionally produces reserved-in-query
/// chars (space, `%`, `&`, `=`, `#`, `+`, `;`, `<`, `>`) and non-ASCII
/// bytes (`Ö`, `ö`, `é`, `日`). Display percent-encodes these, parse
/// percent-decodes them back; the round-trip property is what exercises
/// the encoder pair end-to-end.
fn percent_encoded_value_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        // Original tame shape stays in the mix at higher weight so most
        // generated FlakeRefs still cover the no-encoding-needed path.
        4 => "[a-zA-Z0-9._\\-]{0,16}",
        // Wider shape: lets the round-trip property hit `%`, ` `, etc.
        1 => proptest::string::string_regex("[a-zA-Z0-9 %&=#+;<>Öö\u{e9}\u{65e5}]{1,16}")
            .expect("regex must compile"),
    ]
}

fn fragment_strategy() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        Just(None),
        "[a-zA-Z][a-zA-Z0-9._\\-]{0,15}".prop_map(Some),
        // Fragment-specific: the fragment AsciiSet additionally encodes
        // `:@/?` beyond the query set, so include those alongside the
        // wide-value chars to cover the divergent encoding rules.
        proptest::string::string_regex("[a-zA-Z0-9 %&=#+;<>:@/?Öö\u{e9}\u{65e5}]{1,16}")
            .expect("regex must compile")
            .prop_map(Some),
    ]
}

// The free-form `arbitrary_kv_strategy` was retired when the per-scheme
// query-key allow-list landed: the only keys the parser now lets through
// to `LocationParameters::arbitrary` are scheme-specific (`name` on Git
// and Mercurial, `unpack` on Tarball/File, ...), so a scheme-agnostic
// `x...` strategy would generate inputs that the per-scheme allow-list
// rejects and the round-trip property would fail. Future work: feed the
// chosen `FlakeRefType` into `location_parameters_strategy` and emit
// scheme-allowed extension keys from a per-scheme bag.

/// Per-field strategy for the seven Git-typed params. Bools are
/// represented `Option<bool>` because the parser surfaces typed bools
/// for `lfs`/`exportIgnore`/`allRefs`/`verifyCommit`; the three
/// signature-key params (`keytype`, `publicKey`, `publicKeys`) stay
/// free-form `Option<String>` because Nix does not enumerate valid
/// values.
type GitTypedParams = (
    Option<bool>,
    Option<bool>,
    Option<bool>,
    Option<bool>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn git_typed_params_strategy() -> impl Strategy<Value = GitTypedParams> {
    let key_value = "[a-zA-Z0-9._\\-]{1,16}";
    (
        prop::option::of(any::<bool>()),
        prop::option::of(any::<bool>()),
        prop::option::of(any::<bool>()),
        prop::option::of(any::<bool>()),
        prop::option::of(key_value),
        prop::option::of(key_value),
        prop::option::of(key_value),
    )
}

/// Param generator: every typed slot can be set on every scheme.
/// `LocationParamKeys::FromStr` recognises the same key set regardless
/// of scheme, so any `(key, value)` pair routes into its typed slot and
/// `Display` re-emits it; the round-trip is scheme-independent.
fn location_parameters_strategy() -> impl Strategy<Value = LocationParameters> {
    (
        // `dir` is plain string and the only reasonable target for
        // wide-value coverage among the typed slots, so route it through
        // `percent_encoded_value_strategy` to exercise the encoder on a
        // typed slot too.
        prop::option::of(percent_encoded_value_strategy()),
        // `host` matches Nix's accepted host shape
        // (`[a-zA-Z0-9.-]*`); the parse-time validator rejects anything
        // else, so the generator must not emit `_` or other characters
        // outside the regex.
        prop::option::of("[a-zA-Z0-9.\\-]{1,16}"),
        prop::option::of("sha256-[a-zA-Z0-9_-]{8,32}"),
        prop::option::of("[0-9]{1,12}"),
        prop::option::of("[0-9]{1,8}"),
        // `submodules` and `shallow` are typed `Option<bool>`; the parser
        // routes both through `parse_bool_param` (strict `"1"`/`"0"`), so
        // the strategy emits bools and the call below maps them to the
        // typed slot.
        prop::option::of(any::<bool>()),
        prop::option::of(any::<bool>()),
        git_typed_params_strategy(),
    )
        .prop_map(
            |(dir, host, nar_hash, last_modified, rev_count, submodules, shallow, git_typed)| {
                let mut params = LocationParameters::default();
                if let Some(d) = dir {
                    params.set_dir(Some(d));
                }
                if let Some(h) = host {
                    params.set_host(Some(h));
                }
                if let Some(n) = nar_hash {
                    params.set_nar_hash(Some(n));
                }
                if let Some(lm) = last_modified {
                    params.set_last_modified(Some(lm));
                }
                if let Some(rc) = rev_count {
                    params.set_rev_count(Some(rc));
                }
                if let Some(s) = submodules {
                    params.set_submodules(Some(s));
                }
                if let Some(s) = shallow {
                    params.set_shallow(Some(s));
                }
                let (lfs, export_ignore, all_refs, verify_commit, keytype, public_key, public_keys) =
                    git_typed;
                params.set_lfs(lfs);
                params.set_export_ignore(export_ignore);
                params.set_all_refs(all_refs);
                params.set_verify_commit(verify_commit);
                params.set_keytype(keytype);
                params.set_public_key(public_key);
                params.set_public_keys(public_keys);
                params
            },
        )
}

fn flake_ref_strategy() -> impl Strategy<Value = FlakeRef> {
    (
        kind_strategy(),
        fragment_strategy(),
        location_parameters_strategy(),
    )
        .prop_map(|(kind, fragment, params)| {
            FlakeRef::new(kind)
                .with_fragment(fragment)
                .with_params(params)
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]

    /// `flake_ref.to_string().parse::<FlakeRef>() == Ok(flake_ref)`: every
    /// arbitrary `FlakeRef` survives a `Display` -> `FromStr` round-trip.
    #[test]
    fn roundtrip_via_string(flake_ref in flake_ref_strategy()) {
        let s = flake_ref.to_string();
        let parsed: FlakeRef = s.parse().expect("Display output failed to parse");
        prop_assert_eq!(flake_ref, parsed);
    }

    /// Stronger: `Display` is the canonical form. `parse(Display(x))` then
    /// `Display(...)` reproduces the first `Display` output exactly.
    #[test]
    fn display_is_canonical(flake_ref in flake_ref_strategy()) {
        let s1 = flake_ref.to_string();
        let parsed: FlakeRef = s1.parse().expect("Display output failed to parse");
        let s2 = parsed.to_string();
        prop_assert_eq!(s1, s2);
    }
}
