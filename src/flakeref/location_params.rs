use std::fmt::Display;

use serde::{Deserialize, Serialize};
use winnow::{
    ModalResult, Parser,
    combinator::{repeat, separated_pair},
    error::{StrContext, StrContextValue},
    token::{take_till, take_until},
};

use crate::{
    error::NixUriError,
    flakeref::{encoding, validators::parse_bool_param},
};

/// Query-string parameters that decorate a `FlakeRef`.
///
/// Notably absent: `ref` and `rev`. Those are typed slots on the `FlakeRef`'s
/// kind ([`crate::FlakeRefType::GitForge`] / [`crate::FlakeRefType::Indirect`])
/// there is one source of truth for ref/rev, and it is not here. The parser
/// extracts `?ref=` / `?rev=` values out of the query string and routes them
/// into those typed slots, setting the kind's `RefLocation` to
/// `QueryParameter` so round-trip Display preserves the form.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, serde(deny_unknown_fields))]
#[non_exhaustive]
pub struct LocationParameters {
    /// The subdirectory of the flake in which flake.nix is located. This parameter
    /// enables having multiple flakes in a repository or tarball. The default is the
    /// root directory of the flake.
    dir: Option<String>,
    /// The hash of the NAR serialisation (in SRI format) of the contents of the flake.
    /// This is useful for flake types such as tarballs that lack a unique content
    /// identifier such as a Git commit hash.
    #[serde(rename = "narHash")]
    nar_hash: Option<String>,
    /// Fetch git submodules during clone. Mirrors Nix's git fetcher
    /// `submodules` setting; URL-time coercion is strict (`value == "1"`),
    /// so the parser accepts `"1"` / `"0"` and rejects anything else.
    pub submodules: Option<bool>,
    /// Use a shallow clone (skip git history). Mirrors Nix's git fetcher
    /// `shallow` setting; URL-time coercion follows the same rule as
    /// [`Self::submodules`].
    pub shallow: Option<bool>,
    // Only available to certain types.
    host: Option<String>,
    // Not available to user.
    #[serde(rename = "revCount")]
    rev_count: Option<String>,
    // Not available to user.
    #[serde(rename = "lastModified")]
    last_modified: Option<String>,
    /// Git-LFS support. Boolean values follow Nix's URL-time coercion:
    /// `"1"` -> `true`, `"0"` -> `false`; anything else (including
    /// `"true"` / `"false"`) is rejected at parse time so the diagnostic
    /// stays visible.
    pub lfs: Option<bool>,
    /// Honour `.gitattributes` `export-ignore` directives during fetch.
    #[serde(rename = "exportIgnore")]
    pub export_ignore: Option<bool>,
    /// Fetch all refs, not just the requested one.
    #[serde(rename = "allRefs")]
    pub all_refs: Option<bool>,
    /// Verify the commit signature against the configured key set.
    #[serde(rename = "verifyCommit")]
    pub verify_commit: Option<bool>,
    /// Signature key type (e.g. `ssh-ed25519`). Stored as a free-form
    /// string because Nix does not enumerate valid values.
    pub keytype: Option<String>,
    /// Public key bytes used to verify commit signatures.
    #[serde(rename = "publicKey")]
    pub public_key: Option<String>,
    /// Multi-key bag (Nix uses a single string with platform-specific
    /// delimiters).
    #[serde(rename = "publicKeys")]
    pub public_keys: Option<String>,
    /// Unrecognised query parameters preserved verbatim so `Display` can
    /// round-trip them. Recognised keys (canonical Nix camelCase spellings,
    /// plus `ref`/`rev` which route to the kind's typed slots) are pulled
    /// out by the parser before anything reaches this vec.
    arbitrary: Vec<(String, String)>,
}

/// Ref/rev pulled out of a `?ref=`/`?rev=` query string. Threaded out of the
/// param parsers as a side-channel so the caller can route the value into the
/// `FlakeRef`'s typed `kind.ref_` / `kind.rev` slots instead of stashing it
/// in [`LocationParameters`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ParamRefRev {
    pub r#ref: Option<String>,
    pub rev: Option<String>,
}

impl Display for LocationParameters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut entries = self.entries();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        for (i, (key, value)) in entries.iter().enumerate() {
            if i > 0 {
                write!(f, "&")?;
            }
            write!(
                f,
                "{key}={value}",
                key = encoding::encode_query(key),
                value = encoding::encode_query(value)
            )?;
        }
        Ok(())
    }
}

impl LocationParameters {
    #[allow(dead_code)]
    pub(crate) fn parse(input: &mut &str) -> ModalResult<(Self, ParamRefRev)> {
        let param_values: Vec<(&str, &str)> = repeat(
            0..,
            separated_pair(
                take_until(0.., "="),
                '='.context(StrContext::Expected(StrContextValue::CharLiteral('='))),
                take_till(0.., |c| c == '&' || c == '#'),
            ),
        )
        .context(StrContext::Label("location parameters"))
        .parse_next(input)?;

        let mut params = Self::default();
        let mut ref_rev = ParamRefRev::default();
        for (param, value) in param_values {
            // param can start with "&"
            // TODO: actual error handling instead of unwrapping
            // TODO: allow check of the parameters
            if let Ok(param) = param.parse() {
                match param {
                    LocationParamKeys::Dir => params.set_dir(Some(value.into())),
                    LocationParamKeys::NarHash => params.set_nar_hash(Some(value.into())),
                    LocationParamKeys::LastModified => {
                        params.set_last_modified(Some(value.into()));
                    }
                    LocationParamKeys::RevCount => params.set_rev_count(Some(value.into())),
                    LocationParamKeys::Host => params.set_host(Some(value.into())),
                    LocationParamKeys::Ref => ref_rev.r#ref = Some(value.into()),
                    LocationParamKeys::Rev => ref_rev.rev = Some(value.into()),
                    LocationParamKeys::Submodules => {
                        params.set_submodules(parse_bool_param("submodules", value).ok());
                    }
                    LocationParamKeys::Shallow => {
                        params.set_shallow(parse_bool_param("shallow", value).ok());
                    }
                    LocationParamKeys::Lfs => {
                        params.set_lfs(parse_bool_param("lfs", value).ok());
                    }
                    LocationParamKeys::ExportIgnore => {
                        params.set_export_ignore(parse_bool_param("exportIgnore", value).ok());
                    }
                    LocationParamKeys::AllRefs => {
                        params.set_all_refs(parse_bool_param("allRefs", value).ok());
                    }
                    LocationParamKeys::VerifyCommit => {
                        params.set_verify_commit(parse_bool_param("verifyCommit", value).ok());
                    }
                    LocationParamKeys::Keytype => params.set_keytype(Some(value.into())),
                    LocationParamKeys::PublicKey => params.set_public_key(Some(value.into())),
                    LocationParamKeys::PublicKeys => params.set_public_keys(Some(value.into())),
                    LocationParamKeys::Arbitrary(param) => {
                        params.add_arbitrary((param, value.into()));
                    }
                }
            }
        }
        Ok((params, ref_rev))
    }

    /// Chainable setter for the `dir` parameter; returns `&mut Self` so the
    /// builder form (`params.dir(Some(...)).host(Some(...))`) reads naturally.
    pub fn dir(&mut self, dir: Option<String>) -> &mut Self {
        self.dir = dir;
        self
    }

    /// Chainable setter for the `narHash` parameter. See [`Self::dir`].
    pub fn nar_hash(&mut self, nar_hash: Option<String>) -> &mut Self {
        self.nar_hash = nar_hash;
        self
    }

    /// Chainable setter for the `host` parameter. See [`Self::dir`].
    pub fn host(&mut self, host: Option<String>) -> &mut Self {
        self.host = host;
        self
    }

    /// Replace the `dir` parameter (a flake's subdirectory inside its repo).
    pub fn set_dir(&mut self, dir: Option<String>) {
        self.dir = dir;
    }

    /// Replace the `narHash` parameter (SRI-format NAR hash).
    pub fn set_nar_hash(&mut self, nar_hash: Option<String>) {
        self.nar_hash = nar_hash;
    }

    /// Replace the `host` parameter (used to override the default host for
    /// kinds that resolve to a forge or remote URL).
    pub fn set_host(&mut self, host: Option<String>) {
        self.host = host;
    }

    /// Borrow the `host` query value, when set. The canonical-default
    /// fallback (`github.com` / `gitlab.com` / `git.sr.ht`) is the
    /// `FlakeRef::domain` accessor's job, not this one.
    pub(crate) fn host_value(&self) -> Option<&str> {
        self.host.as_deref()
    }

    /// Borrow the `narHash` query value, when set. Used by
    /// [`crate::FlakeRef::to_canonical_string`] to match the Nix schemes
    /// that emit the SRI hash on canonical URLs (git-archive forges and
    /// the curl-based tarball/file scheme).
    pub(crate) fn nar_hash_value(&self) -> Option<&str> {
        self.nar_hash.as_deref()
    }

    /// Whether `?submodules=` carries the truthy `"1"` value. Used to
    /// gate canonical emission: Nix writes `?submodules=1` only for
    /// the truthy branch. The slot is typed `Option<bool>` because
    /// URL-time bool coercion follows the strict `value == "1"` rule.
    pub(crate) fn submodules_truthy(&self) -> bool {
        self.submodules.unwrap_or(false)
    }

    /// Whether `?shallow=` carries the truthy `"1"` value. Companion to
    /// [`Self::submodules_truthy`].
    pub(crate) fn shallow_truthy(&self) -> bool {
        self.shallow.unwrap_or(false)
    }

    /// Mutable handle to the `revCount` slot for in-place edits.
    pub fn rev_count_mut(&mut self) -> &mut Option<String> {
        &mut self.rev_count
    }

    /// Replace the `lastModified` parameter (Unix timestamp of the source).
    pub fn set_last_modified(&mut self, last_modified: Option<String>) {
        self.last_modified = last_modified;
    }

    /// Replace the `revCount` parameter (commit count from the repo root).
    pub fn set_rev_count(&mut self, rev_count: Option<String>) {
        self.rev_count = rev_count;
    }

    /// Replace the `submodules` boolean parameter.
    pub fn set_submodules(&mut self, submodules: Option<bool>) {
        self.submodules = submodules;
    }

    /// Replace the `shallow` boolean parameter.
    pub fn set_shallow(&mut self, shallow: Option<bool>) {
        self.shallow = shallow;
    }

    /// Replace the `lfs` boolean parameter.
    pub fn set_lfs(&mut self, lfs: Option<bool>) {
        self.lfs = lfs;
    }

    /// Replace the `exportIgnore` boolean parameter.
    pub fn set_export_ignore(&mut self, export_ignore: Option<bool>) {
        self.export_ignore = export_ignore;
    }

    /// Replace the `allRefs` boolean parameter.
    pub fn set_all_refs(&mut self, all_refs: Option<bool>) {
        self.all_refs = all_refs;
    }

    /// Replace the `verifyCommit` boolean parameter.
    pub fn set_verify_commit(&mut self, verify_commit: Option<bool>) {
        self.verify_commit = verify_commit;
    }

    /// Replace the `keytype` parameter (signature key type).
    pub fn set_keytype(&mut self, keytype: Option<String>) {
        self.keytype = keytype;
    }

    /// Replace the `publicKey` parameter.
    pub fn set_public_key(&mut self, public_key: Option<String>) {
        self.public_key = public_key;
    }

    /// Replace the `publicKeys` parameter.
    pub fn set_public_keys(&mut self, public_keys: Option<String>) {
        self.public_keys = public_keys;
    }

    /// Append a `(key, value)` pair to the unrecognised-parameter vec.
    /// Used by the parser for keys that do not match a typed slot; rendered
    /// verbatim by `Display` to keep round-trip parity.
    pub fn add_arbitrary(&mut self, arbitrary: (String, String)) {
        self.arbitrary.push(arbitrary);
    }

    /// Every set query parameter as a `(key, value)` pair: the populated
    /// typed slots followed by the arbitrary key/value bag, in storage order.
    /// Callers that emit a query string (`Display` here, `FlakeRef`'s combined
    /// ref/rev + params block) sort the merged list by key to match Nix's
    /// alphabetical emission order.
    pub(crate) fn entries(&self) -> Vec<(&str, &str)> {
        let mut entries: Vec<(&str, &str)> = Vec::new();
        if let Some(v) = &self.dir {
            entries.push(("dir", v));
        }
        if let Some(v) = &self.host {
            entries.push(("host", v));
        }
        if let Some(v) = &self.nar_hash {
            entries.push(("narHash", v));
        }
        if let Some(v) = &self.last_modified {
            entries.push(("lastModified", v));
        }
        if let Some(v) = &self.rev_count {
            entries.push(("revCount", v));
        }
        if let Some(v) = self.submodules {
            entries.push(("submodules", bool_repr(v)));
        }
        if let Some(v) = self.shallow {
            entries.push(("shallow", bool_repr(v)));
        }
        if let Some(v) = self.lfs {
            entries.push(("lfs", bool_repr(v)));
        }
        if let Some(v) = self.export_ignore {
            entries.push(("exportIgnore", bool_repr(v)));
        }
        if let Some(v) = self.all_refs {
            entries.push(("allRefs", bool_repr(v)));
        }
        if let Some(v) = self.verify_commit {
            entries.push(("verifyCommit", bool_repr(v)));
        }
        if let Some(v) = &self.keytype {
            entries.push(("keytype", v));
        }
        if let Some(v) = &self.public_key {
            entries.push(("publicKey", v));
        }
        if let Some(v) = &self.public_keys {
            entries.push(("publicKeys", v));
        }
        for (k, v) in &self.arbitrary {
            entries.push((k.as_str(), v.as_str()));
        }
        entries
    }
}

/// Canonical wire form for a boolean param: `"1"` for true, `"0"` for false.
/// Matches Nix's URL-time coercion, which treats only `"1"` as true. The
/// parser accepts the same two literals; Display picks the canonical
/// spelling so a re-parse is byte-stable.
fn bool_repr(b: bool) -> &'static str {
    if b { "1" } else { "0" }
}

#[non_exhaustive]
pub(crate) enum LocationParamKeys {
    Dir,
    NarHash,
    LastModified,
    RevCount,
    Host,
    Ref,
    Rev,
    Submodules,
    Shallow,
    Lfs,
    ExportIgnore,
    AllRefs,
    VerifyCommit,
    Keytype,
    PublicKey,
    PublicKeys,
    Arbitrary(String),
}

impl std::str::FromStr for LocationParamKeys {
    type Err = NixUriError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "dir" | "&dir" => Ok(Self::Dir),
            "narHash" | "&narHash" => Ok(Self::NarHash),
            "lastModified" | "&lastModified" => Ok(Self::LastModified),
            "revCount" | "&revCount" => Ok(Self::RevCount),
            "host" | "&host" => Ok(Self::Host),
            "rev" | "&rev" => Ok(Self::Rev),
            "ref" | "&ref" => Ok(Self::Ref),
            "submodules" | "&submodules" => Ok(Self::Submodules),
            "shallow" | "&shallow" => Ok(Self::Shallow),
            "lfs" | "&lfs" => Ok(Self::Lfs),
            "exportIgnore" | "&exportIgnore" => Ok(Self::ExportIgnore),
            "allRefs" | "&allRefs" => Ok(Self::AllRefs),
            "verifyCommit" | "&verifyCommit" => Ok(Self::VerifyCommit),
            "keytype" | "&keytype" => Ok(Self::Keytype),
            "publicKey" | "&publicKey" => Ok(Self::PublicKey),
            "publicKeys" | "&publicKeys" => Ok(Self::PublicKeys),
            // The typed arms above strip the leading `&` that the param
            // parser leaves in place between adjacent k=v pairs; do the
            // same for the arbitrary fallback so `?xA=&xB=` round-trips.
            arbitrary => Ok(Self::Arbitrary(
                arbitrary.strip_prefix('&').unwrap_or(arbitrary).into(),
            )),
        }
    }
}

#[cfg(test)]
mod inc_parse {
    use super::*;
    #[test]
    fn no_str() {
        let expected = LocationParameters::default();
        let in_str = "";
        let (outstr, (parsed_param, ref_rev)) =
            LocationParameters::parse.parse_peek(in_str).unwrap();
        assert_eq!("", outstr);
        assert_eq!(expected, parsed_param);
        assert_eq!(ref_rev, ParamRefRev::default());
    }
    #[test]
    fn empty() {
        let expected = LocationParameters::default();
        let in_str = "";
        let (rest, (output, ref_rev)) = LocationParameters::parse.parse_peek(in_str).unwrap();
        assert_eq!("", rest);
        assert_eq!(output, expected);
        assert_eq!(ref_rev, ParamRefRev::default());
    }
    #[test]
    fn empty_hash_terminated() {
        let expected = LocationParameters::default();
        let in_str = "#";
        let (rest, (output, ref_rev)) = LocationParameters::parse.parse_peek(in_str).unwrap();
        assert_eq!("#", rest);
        assert_eq!(output, expected);
        assert_eq!(ref_rev, ParamRefRev::default());
    }
    #[test]
    fn dir() {
        let mut expected = LocationParameters::default();
        expected.dir(Some("foo".to_string()));

        let in_str = "dir=foo";
        let (rest, (output, _)) = LocationParameters::parse.parse_peek(in_str).unwrap();
        assert_eq!("", rest);
        assert_eq!(output, expected);

        let in_str = "&dir=foo";
        let (rest, (output, _)) = LocationParameters::parse.parse_peek(in_str).unwrap();
        assert_eq!("", rest);
        assert_eq!(output, expected);
        let in_str = "dir=&dir=foo";
        let (rest, (output, _)) = LocationParameters::parse.parse_peek(in_str).unwrap();
        assert_eq!("", rest);
        assert_eq!(output, expected);

        expected.dir(Some(String::new()));
        let in_str = "dir=";
        let (rest, (output, _)) = LocationParameters::parse.parse_peek(in_str).unwrap();
        assert_eq!("", rest);
        assert_eq!(output, expected);
    }
    #[test]
    fn dir_hash_term() {
        let mut expected = LocationParameters::default();
        expected.dir(Some("foo".to_string()));

        let in_str = "dir=foo#fizz";
        let (rest, (output, _)) = LocationParameters::parse.parse_peek(in_str).unwrap();
        assert_eq!("#fizz", rest);
        assert_eq!(output, expected);

        let in_str = "&dir=foo#fizz";
        let (rest, (output, _)) = LocationParameters::parse.parse_peek(in_str).unwrap();
        assert_eq!("#fizz", rest);
        assert_eq!(output, expected);
        let in_str = "dir=&dir=foo#fizz";
        let (rest, (output, _)) = LocationParameters::parse.parse_peek(in_str).unwrap();
        assert_eq!("#fizz", rest);
        assert_eq!(output, expected);

        expected.dir(Some(String::new()));
        let in_str = "dir=#fizz";
        let (rest, (output, _)) = LocationParameters::parse.parse_peek(in_str).unwrap();
        assert_eq!("#fizz", rest);
        assert_eq!(output, expected);
    }

    #[test]
    fn canonical_param_keys_round_trip() {
        // Canonical Nix spells these in camelCase; only the camelCase
        // form routes into the typed slots.
        let mut expected = LocationParameters::default();
        expected.set_nar_hash(Some("sha256-abc".into()));
        expected.set_last_modified(Some("12345".into()));
        expected.set_rev_count(Some("42".into()));

        let in_str = "narHash=sha256-abc&lastModified=12345&revCount=42";
        let (rest, (output, _)) = LocationParameters::parse.parse_peek(in_str).unwrap();
        assert_eq!("", rest);
        assert_eq!(output, expected);

        // Display emits the canonical (camelCase) spelling in alphabetical
        // key order: lastModified, narHash, revCount.
        assert_eq!(
            output.to_string(),
            "lastModified=12345&narHash=sha256-abc&revCount=42"
        );
    }

    #[test]
    fn snake_case_falls_through_to_arbitrary() {
        // The snake_case spellings (`nar_hash`, `last_modified`, `rev_count`)
        // do not match a typed slot; they fall through to `arbitrary`.
        let in_str = "nar_hash=sha256-abc";
        let (rest, (output, _)) = LocationParameters::parse.parse_peek(in_str).unwrap();
        assert_eq!("", rest);
        // The typed nar_hash slot is still empty; the value lives in arbitrary.
        let mut expected = LocationParameters::default();
        expected.add_arbitrary(("nar_hash".into(), "sha256-abc".into()));
        assert_eq!(output, expected);
    }
}

#[cfg(test)]
mod git_typed_params {
    //! Pin the seven Git-flavoured params from Nix's git fetcher settings
    //! to typed slots on `LocationParameters` so they route into the
    //! typed slots rather than the arbitrary bag.

    use crate::{FlakeRef, NixUriError};
    use rstest::rstest;

    #[rstest]
    #[case("1", true)]
    #[case("0", false)]
    fn lfs_accepts_bool_forms(#[case] input: &str, #[case] expected: bool) {
        let url = format!("git+ssh://example.com/repo?lfs={input}");
        let parsed: FlakeRef = url.parse().unwrap();
        assert_eq!(parsed.params().lfs, Some(expected));
    }

    #[rstest]
    #[case("1", true)]
    #[case("0", false)]
    fn export_ignore_accepts_bool_forms(#[case] input: &str, #[case] expected: bool) {
        let url = format!("git+ssh://example.com/repo?exportIgnore={input}");
        let parsed: FlakeRef = url.parse().unwrap();
        assert_eq!(parsed.params().export_ignore, Some(expected));
    }

    #[rstest]
    #[case("1", true)]
    #[case("0", false)]
    fn all_refs_accepts_bool_forms(#[case] input: &str, #[case] expected: bool) {
        let url = format!("git+ssh://example.com/repo?allRefs={input}");
        let parsed: FlakeRef = url.parse().unwrap();
        assert_eq!(parsed.params().all_refs, Some(expected));
    }

    #[rstest]
    #[case("1", true)]
    #[case("0", false)]
    fn verify_commit_accepts_bool_forms(#[case] input: &str, #[case] expected: bool) {
        let url = format!("git+ssh://example.com/repo?verifyCommit={input}");
        let parsed: FlakeRef = url.parse().unwrap();
        assert_eq!(parsed.params().verify_commit, Some(expected));
    }

    #[rstest]
    #[case("lfs", "yes")]
    #[case("exportIgnore", "yes")]
    #[case("allRefs", "yes")]
    #[case("verifyCommit", "yes")]
    // Nix's URL-time coercion is strictly `value == "1"`; `"true"` /
    // `"false"` look right but Nix maps anything other than `"1"` to
    // false. Reject them at parse time so the diagnostic surfaces
    // instead of silently flipping the bool.
    #[case("lfs", "true")]
    #[case("exportIgnore", "false")]
    #[case("allRefs", "True")]
    #[case("verifyCommit", "TRUE")]
    fn bool_keys_reject_non_bool_values(#[case] key: &str, #[case] value: &str) {
        let url = format!("git+ssh://example.com/repo?{key}={value}");
        let err = url.parse::<FlakeRef>().unwrap_err();
        match err {
            NixUriError::InvalidValue { field, .. } => {
                assert_eq!(field, key, "expected error.field to name the rejected key");
            }
            other => panic!("expected InvalidValue, got {other:?}"),
        }
    }

    #[test]
    fn keytype_routes_to_typed_slot() {
        let parsed: FlakeRef = "git+ssh://example.com/repo?keytype=ssh-ed25519"
            .parse()
            .unwrap();
        assert_eq!(parsed.params().keytype.as_deref(), Some("ssh-ed25519"));
    }

    #[test]
    fn public_key_routes_to_typed_slot() {
        let parsed: FlakeRef = "git+ssh://example.com/repo?publicKey=abcdef"
            .parse()
            .unwrap();
        assert_eq!(parsed.params().public_key.as_deref(), Some("abcdef"));
    }

    #[test]
    fn public_keys_routes_to_typed_slot() {
        let parsed: FlakeRef = "git+ssh://example.com/repo?publicKeys=k1.k2.k3"
            .parse()
            .unwrap();
        assert_eq!(parsed.params().public_keys.as_deref(), Some("k1.k2.k3"));
    }

    #[test]
    fn display_emits_seven_keys_alphabetically() {
        // Alphabetical (ASCII) order across the seven new keys plus a
        // pre-existing `narHash` entry: allRefs, exportIgnore, keytype,
        // lfs, narHash, publicKey, publicKeys, verifyCommit.
        let url = "git+ssh://example.com/repo?\
                   verifyCommit=1&publicKeys=k1.k2&publicKey=abc&\
                   narHash=sha256-x&lfs=1&keytype=ssh-ed25519&\
                   exportIgnore=0&allRefs=1";
        let parsed: FlakeRef = url.parse().unwrap();
        let expected = "git+ssh://example.com/repo?\
                        allRefs=1&exportIgnore=0&keytype=ssh-ed25519&\
                        lfs=1&narHash=sha256-x&publicKey=abc&\
                        publicKeys=k1.k2&verifyCommit=1";
        assert_eq!(parsed.to_string(), expected);
    }

    #[rstest]
    #[case("1", true)]
    #[case("0", false)]
    fn submodules_accepts_bool_forms(#[case] input: &str, #[case] expected: bool) {
        let url = format!("git+ssh://example.com/repo?submodules={input}");
        let parsed: FlakeRef = url.parse().unwrap();
        assert_eq!(parsed.params().submodules, Some(expected));
    }

    #[rstest]
    #[case("1", true)]
    #[case("0", false)]
    fn shallow_accepts_bool_forms(#[case] input: &str, #[case] expected: bool) {
        let url = format!("git+ssh://example.com/repo?shallow={input}");
        let parsed: FlakeRef = url.parse().unwrap();
        assert_eq!(parsed.params().shallow, Some(expected));
    }

    #[rstest]
    #[case("submodules", "garbage")]
    #[case("submodules", "true")]
    #[case("shallow", "garbage")]
    #[case("shallow", "false")]
    fn submodules_shallow_reject_non_bool_values(#[case] key: &str, #[case] value: &str) {
        // submodules and shallow are bool-coerced by Nix; inputs that
        // are not `"1"` / `"0"` surface as InvalidValue with a field
        // tag.
        let url = format!("git+ssh://example.com/repo?{key}={value}");
        let err = url.parse::<FlakeRef>().unwrap_err();
        match err {
            NixUriError::InvalidValue { field, .. } => assert_eq!(field, key),
            other => panic!("expected InvalidValue, got {other:?}"),
        }
    }

    #[test]
    fn submodules_round_trips_canonically() {
        let url = "git+ssh://example.com/repo?submodules=1";
        let parsed: FlakeRef = url.parse().unwrap();
        assert_eq!(parsed.to_string(), url);
        assert_eq!(parsed.params().submodules, Some(true));
    }

    #[test]
    fn shallow_round_trips_canonically() {
        let url = "git+ssh://example.com/repo?shallow=1";
        let parsed: FlakeRef = url.parse().unwrap();
        assert_eq!(parsed.to_string(), url);
        assert_eq!(parsed.params().shallow, Some(true));
    }

    #[test]
    fn round_trip_realistic_git_url() {
        let url = "git+ssh://example.com/repo?allRefs=1&lfs=1&publicKey=abc";
        let parsed: FlakeRef = url.parse().unwrap();
        assert_eq!(parsed.params().all_refs, Some(true));
        assert_eq!(parsed.params().lfs, Some(true));
        assert_eq!(parsed.params().public_key.as_deref(), Some("abc"));
        assert_eq!(parsed.to_string(), url);
    }
}
