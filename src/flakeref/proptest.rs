//! Recursive proptest generators and properties for [`FlakeRef`].
//!
//! Every generated value is one Nix accepts: legal ref names, SHA-1 revisions where a
//! forge or indirect reference needs them, URLs in the form Nix stores them, and only the
//! attributes each type takes.

use std::borrow::Cow;

use proptest::prelude::*;

use super::{
    Download, FlakeRef, FlakeRefType, GitForge, GitForgePlatform, GitRepo, Indirect, MercurialRepo,
    PathRef,
    url::{Authority, Host, Url},
    validators::{is_legal_ref_name, is_sha1_hex, matches_ref_shape},
};
use crate::Context;

fn ref_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => "[a-zA-Z][a-zA-Z0-9_.\\-]{0,15}",
        1 => "[a-zA-Z][a-zA-Z0-9_\\-]{0,7}/[a-zA-Z][a-zA-Z0-9_\\-]{0,7}",
    ]
    .prop_filter("legal ref, not a rev", |s: &String| {
        is_legal_ref_name(s) && matches_ref_shape(s) && !is_sha1_hex(s)
    })
}

fn rev_strategy(expressible: bool) -> BoxedStrategy<String> {
    if expressible {
        "[0-9a-f]{40}".boxed()
    } else {
        prop_oneof!["[0-9a-f]{40}", "[0-9A-F]{40}"].boxed()
    }
}

fn opt<T: std::fmt::Debug + Clone + 'static>(
    strategy: impl Strategy<Value = T> + 'static,
) -> BoxedStrategy<Option<T>> {
    prop_oneof![Just(None), strategy.prop_map(Some)].boxed()
}

fn lock_only<T: std::fmt::Debug + Clone + 'static>(
    expressible: bool,
    strategy: impl Strategy<Value = T> + 'static,
) -> BoxedStrategy<Option<T>> {
    if expressible {
        Just(None).boxed()
    } else {
        opt(strategy)
    }
}

fn nar_hash_strategy() -> impl Strategy<Value = String> {
    "sha256-[A-Za-z0-9+/]{42}[AEIMQUYcgkosw048]="
}

fn query_value_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => "[a-zA-Z0-9._\\-]{0,16}",
        1 => proptest::string::string_regex("[a-zA-Z0-9 %&=#+;<>/:@?Öö\u{e9}\u{65e5}]{1,16}")
            .expect("regex must compile"),
    ]
}

fn platform_strategy() -> impl Strategy<Value = GitForgePlatform> {
    prop_oneof![
        Just(GitForgePlatform::GitHub),
        Just(GitForgePlatform::GitLab),
        Just(GitForgePlatform::SourceHut),
    ]
}

fn owner_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => "[a-zA-Z~][a-zA-Z0-9_.!*-]{0,15}",
        1 => proptest::collection::vec("[a-zA-Z][a-zA-Z0-9_-]{0,7}", 2..=3)
            .prop_map(|segments| segments.join("/")),
    ]
}

fn git_forge_strategy(expressible: bool) -> impl Strategy<Value = FlakeRefType> {
    (
        platform_strategy(),
        owner_strategy(),
        "[a-zA-Z][a-zA-Z0-9_.-]{0,15}",
        prop_oneof![
            Just((None, None)),
            ref_strategy().prop_map(|r| (Some(r), None)),
            rev_strategy(expressible).prop_map(|r| (None, Some(r))),
        ],
        opt("[a-zA-Z0-9.\\-]{1,16}"),
        opt(nar_hash_strategy()),
        lock_only(expressible, 0..2_000_000_000u64),
        lock_only(expressible, "[0-9a-f]{40}"),
    )
        .prop_map(
            |(platform, owner, repo, (ref_, rev), host, nar_hash, last_modified, tree_hash)| {
                let mut forge = GitForge::new(platform, owner, repo);
                forge.ref_ = ref_;
                forge.rev = rev;
                forge.host = host;
                forge.nar_hash = nar_hash;
                forge.last_modified = last_modified;
                forge.tree_hash = tree_hash;
                FlakeRefType::GitForge(forge)
            },
        )
}

fn indirect_strategy(expressible: bool) -> impl Strategy<Value = FlakeRefType> {
    (
        "[a-zA-Z][a-zA-Z0-9_-]{0,15}",
        opt(ref_strategy()),
        opt("[0-9a-f]{40}"),
        lock_only(expressible, nar_hash_strategy()),
    )
        .prop_map(|(id, ref_, rev, nar_hash)| {
            let mut indirect = Indirect::new(id);
            indirect.ref_ = ref_;
            indirect.rev = rev;
            indirect.nar_hash = nar_hash;
            FlakeRefType::Indirect(indirect)
        })
}

fn path_strategy() -> impl Strategy<Value = FlakeRefType> {
    let segment = proptest::string::string_regex("[a-zA-Z0-9._ %?#:@\u{e9}-]{1,8}")
        .expect("regex must compile");
    let path = (
        prop_oneof![Just(""), Just("/"), Just("./")],
        proptest::collection::vec(segment, 1..4),
        any::<bool>(),
    )
        .prop_map(|(lead, segments, trailing)| {
            let mut path = format!("{lead}{}", segments.join("/"));
            if trailing {
                path.push('/');
            }
            path
        });
    (
        path,
        opt(query_value_strategy()),
        opt(0..1_000_000u64),
        opt(0..2_000_000_000u64),
        opt(nar_hash_strategy()),
    )
        .prop_map(|(path, rev, rev_count, last_modified, nar_hash)| {
            let mut path = PathRef::new(path);
            path.rev = rev;
            path.rev_count = rev_count;
            path.last_modified = last_modified;
            path.nar_hash = nar_hash;
            FlakeRefType::Path(path)
        })
}

fn url_strategy(schemes: &'static [&'static str]) -> impl Strategy<Value = String> {
    (
        proptest::sample::select(schemes),
        opt("[a-z]{1,8}"),
        "[a-z][a-z0-9.-]{0,11}",
        opt(1..=65535u16),
        proptest::collection::vec(
            proptest::string::string_regex("[a-zA-Z0-9._~*!$ -]{0,8}").expect("regex must compile"),
            0..4,
        ),
        proptest::collection::btree_map("x[a-zA-Z]{0,6}", query_value_strategy(), 0..3),
    )
        .prop_map(|(scheme, user, host, port, segments, query)| {
            let local = scheme == "file";
            let mut path = vec![Cow::Borrowed("")];
            path.extend(segments.into_iter().map(Cow::Owned));
            let url = Url {
                scheme,
                authority: Some(Authority {
                    user: if local { None } else { user.map(Cow::Owned) },
                    password: None,
                    host: Host::Name(if local {
                        Cow::Borrowed("")
                    } else {
                        host.into()
                    }),
                    port: if local { None } else { port },
                }),
                path,
                query: query
                    .into_iter()
                    .map(|(key, value)| (Cow::Owned(key), Cow::Owned(value)))
                    .collect(),
            };
            // Rendering is not yet the normal form (`file://` stores as `file:///`), parsing the rendering is.
            Url::parse(&url.to_string()).unwrap().to_string()
        })
}

fn flag_strategy(expressible: bool) -> BoxedStrategy<Option<bool>> {
    if expressible {
        prop_oneof![Just(None), Just(Some(true))].boxed()
    } else {
        opt(any::<bool>())
    }
}

fn git_strategy(expressible: bool) -> impl Strategy<Value = FlakeRefType> {
    (
        url_strategy(&["https", "http", "ssh", "file", "git", "foo"]),
        opt(ref_strategy()),
        opt(rev_strategy(expressible)),
        (
            flag_strategy(expressible),
            flag_strategy(expressible),
            flag_strategy(expressible),
            flag_strategy(expressible),
            lock_only(expressible, any::<bool>()),
        ),
        (
            lock_only(expressible, nar_hash_strategy()),
            lock_only(expressible, 0..1_000_000u64),
            lock_only(expressible, 0..2_000_000_000u64),
            lock_only(expressible, "[a-z]{1,8}"),
            lock_only(expressible, "[0-9a-f]{40}-dirty"),
        ),
    )
        .prop_map(
            |(
                url,
                ref_,
                rev,
                (shallow, submodules, lfs, export_ignore, all_refs),
                (nar_hash, rev_count, last_modified, name, dirty_rev),
            )| {
                let mut git = GitRepo::new(url);
                git.ref_ = ref_;
                git.rev = rev;
                git.shallow = shallow;
                git.submodules = submodules;
                git.lfs = lfs;
                git.export_ignore = export_ignore;
                git.all_refs = all_refs;
                git.nar_hash = nar_hash;
                git.rev_count = rev_count;
                git.last_modified = last_modified;
                git.name = name;
                git.dirty_short_rev = dirty_rev.as_ref().map(|r| format!("{}-dirty", &r[..7]));
                git.dirty_rev = dirty_rev;
                FlakeRefType::Git(git)
            },
        )
}

fn mercurial_strategy(expressible: bool) -> impl Strategy<Value = FlakeRefType> {
    (
        url_strategy(&["https", "http", "ssh", "file"]),
        opt(ref_strategy()),
        opt(rev_strategy(expressible)),
        lock_only(expressible, 0..1_000_000u64),
        lock_only(expressible, nar_hash_strategy()),
        lock_only(expressible, "[a-z]{1,8}"),
    )
        .prop_map(|(url, ref_, rev, rev_count, nar_hash, name)| {
            let mut hg = MercurialRepo::new(url);
            hg.ref_ = ref_;
            hg.rev = rev;
            hg.rev_count = rev_count;
            hg.nar_hash = nar_hash;
            hg.name = name;
            FlakeRefType::Mercurial(hg)
        })
}

fn download_strategy(expressible: bool) -> impl Strategy<Value = FlakeRefType> {
    (
        url_strategy(&["https", "http", "file"]),
        opt(nar_hash_strategy()),
        lock_only(expressible, "[0-9a-f]{40}"),
        lock_only(expressible, 0..1_000_000u64),
        lock_only(expressible, 0..2_000_000_000u64),
        lock_only(expressible, "[a-z]{1,8}"),
        lock_only(expressible, any::<bool>()),
        if expressible {
            Just(false).boxed()
        } else {
            any::<bool>().boxed()
        },
    )
        .prop_map(
            |(url, nar_hash, rev, rev_count, last_modified, name, unpack, file)| {
                let mut download = Download::new(url);
                download.nar_hash = nar_hash;
                download.rev = rev;
                download.rev_count = rev_count;
                download.last_modified = last_modified;
                download.name = name;
                download.unpack = unpack;
                if file {
                    FlakeRefType::File(download)
                } else {
                    FlakeRefType::Tarball(download)
                }
            },
        )
}

fn kind_strategy(expressible: bool) -> impl Strategy<Value = FlakeRefType> {
    prop_oneof![
        git_forge_strategy(expressible),
        indirect_strategy(expressible),
        path_strategy(),
        git_strategy(expressible),
        mercurial_strategy(expressible),
        download_strategy(expressible),
    ]
}

fn flake_ref_strategy(expressible: bool) -> impl Strategy<Value = FlakeRef> {
    (kind_strategy(expressible), opt(query_value_strategy()))
        .prop_map(|(kind, dir)| FlakeRef::new(kind).with_dir(dir))
}

fn parse(s: &str) -> FlakeRef {
    FlakeRef::parse(s, Context::Standalone)
        .unwrap_or_else(|e| panic!("Display output {s:?} failed to parse: {e:?}"))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]

    /// The attributes carry every field.
    #[test]
    fn attrs_round_trip(flake_ref in flake_ref_strategy(false)) {
        let attrs = flake_ref.to_attrs();
        prop_assert_eq!(FlakeRef::from_attrs(&attrs).unwrap(), flake_ref);
    }

    /// So does the JSON of a lock file node.
    #[cfg(feature = "serde")]
    #[test]
    fn json_round_trip(flake_ref in flake_ref_strategy(false)) {
        let json = serde_json::to_string(&flake_ref).unwrap();
        prop_assert_eq!(serde_json::from_str::<FlakeRef>(&json).unwrap(), flake_ref);
    }

    /// A value whose attributes a URL can carry survives `Display`.
    #[test]
    fn display_round_trip(flake_ref in flake_ref_strategy(true)) {
        prop_assert_eq!(parse(&flake_ref.to_string()), flake_ref);
    }

    /// `Display` is a normal form: parsing it and rendering again gives the same string.
    #[test]
    fn display_is_idempotent(flake_ref in flake_ref_strategy(false)) {
        let rendered = flake_ref.to_string();
        prop_assert_eq!(parse(&rendered).to_string(), rendered);
    }
}
