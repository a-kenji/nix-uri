use std::fmt::Display;

use serde::{Deserialize, Serialize};

use crate::error::{NixUriError, UnsupportedReason};

pub(crate) mod encoding;
mod fr_type;
pub use fr_type::FlakeRefType;
pub(crate) mod location_params;
pub(crate) use location_params::LocationParamKeys;
pub use location_params::LocationParameters;
mod transport_layer;
pub use transport_layer::TransportLayer;
mod forge;
pub use forge::{GitForge, GitForgePlatform};
mod resource_url;
pub use resource_url::{ResourceType, ResourceUrl};
#[cfg(test)]
mod proptest;
pub(crate) mod validators;

/// Names where a ref or rev is rendered in a `FlakeRef`.
///
/// `RefLocation` is a routing tag, not a presence flag: a `FlakeRef` whose
/// kind has no ref and no rev still carries a `RefLocation` saying where one
/// *would* be written if it were set. The "no ref or rev" state is encoded by
/// `ref_ == None && rev == None` on the kind, so a `RefLocation::None`
/// variant would be redundant and is therefore unrepresentable.
///
/// `Default` is `PathComponent` because that is the canonical form for the
/// ref-bearing kinds (`GitForge`, `Indirect`); the parser flips this to
/// `QueryParameter` when the value arrived via `?ref=` / `?rev=`, preserving
/// the round-trip shape.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum RefLocation {
    /// Rendered in the path component, e.g. `github:owner/repo/<value>`.
    #[default]
    PathComponent,
    /// Rendered as a query parameter, e.g. `?ref=<value>` or `?rev=<value>`.
    QueryParameter,
}

/// The General Flake Ref Schema
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, serde(deny_unknown_fields))]
#[non_exhaustive]
pub struct FlakeRef {
    /// The shape of the URL: a git forge, a resource (`git+https://...`,
    /// `tarball+https://...`, ...), an indirect (registry) ref, or a path.
    /// Reachable through [`Self::kind`] and the consuming [`Self::with_kind`]
    /// builder; private so the public surface stays consistent with
    /// `fragment` and `params`, which only expose accessors.
    pub(crate) kind: FlakeRefType,
    fragment: Option<String>,
    params: Box<LocationParameters>,
}

/// Identity of a git-forge flake ref: `(platform, owner, repo, domain)`.
///
/// Returned by [`FlakeRef::forge_identity`] for kinds that resolve to a
/// git forge (`GitForge` always; `Resource(Git)` does not; its
/// owner/repo/domain are extracted ad-hoc from a URL string and not
/// guaranteed to be present, so it returns `None` for `forge_identity`).
///
/// `#[non_exhaustive]` reserves room for future fields without breaking
/// downstream match arms.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ForgeIdentity {
    pub platform: GitForgePlatform,
    pub owner: String,
    pub repo: String,
    pub domain: String,
}

/// Discriminates the four ref/rev presence states without forcing callers
/// to read both [`FlakeRef::ref_`] and [`FlakeRef::rev`] and reason about
/// the cross product.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RefKind {
    /// Neither `ref_` nor `rev` is set.
    None,
    /// Only `ref_` is set (a branch or tag name, not a 40-hex commit hash).
    Ref,
    /// Only `rev` is set (pinned to a 40-hex commit hash).
    Rev,
    /// Both `ref_` and `rev` are set (canonical Nix's three-segment Indirect
    /// form, `flake:id/ref/rev`).
    Both,
}

impl FlakeRef {
    /// Construct a `FlakeRef` around `kind` with no fragment and an empty
    /// [`LocationParameters`] block. Use the consuming `with_*` builders or
    /// the `set_*` mutators to fill in the rest.
    pub fn new(kind: FlakeRefType) -> Self {
        Self {
            kind,
            ..Self::default()
        }
    }

    /// Read access to the kind. Pattern-matching consumers go through
    /// this accessor rather than the (private) field directly.
    pub fn kind(&self) -> &FlakeRefType {
        &self.kind
    }

    /// Mutable access to the kind for in-place edits. The consuming
    /// [`Self::with_kind`] builder is the right tool when chaining;
    /// reach for `kind_mut` only when you must rewrite a field on a
    /// borrowed `FlakeRef`.
    pub fn kind_mut(&mut self) -> &mut FlakeRefType {
        &mut self.kind
    }

    /// The repo identifier for the kind (`repo` for `GitForge`, the trailing
    /// path segment of a `Resource(Git)` URL); `None` otherwise.
    pub fn id(&self) -> Option<&str> {
        self.kind().id()
    }

    /// Owner (organisation/user) for the kind, when applicable. Returns
    /// `Some` for `GitForge` and for `Resource(Git)` URLs that look like
    /// `domain/owner/repo`; `None` for `Path`, `Indirect`, and other
    /// resources.
    pub fn owner(&self) -> Option<&str> {
        self.kind().owner()
    }

    /// Repository name for the kind, when applicable. See [`Self::owner`]
    /// for the kinds that produce `Some`.
    pub fn repo(&self) -> Option<&str> {
        self.kind().repo()
    }

    /// Domain (host) for the kind, when applicable. Returns the canonical
    /// host string for git-forge platforms (`github.com`, `gitlab.com`,
    /// `git.sr.ht`) -- or the `?host=` override for self-hosted instances
    /// -- and the host portion of a `Resource(Git)` URL (with `:port`
    /// retained when the port is non-default for the scheme); `None`
    /// otherwise.
    ///
    /// Matches Nix's per-scheme host resolution: the `host` attr
    /// defaults to the canonical domain.
    pub fn domain(&self) -> Option<&str> {
        if matches!(self.kind(), FlakeRefType::GitForge(_)) {
            if let Some(host) = self.params.host_value() {
                return Some(host);
            }
        }
        self.kind().domain()
    }

    /// Bundled identity for git-forge kinds. Returns `Some` only for
    /// [`FlakeRefType::GitForge`]; `Resource(Git)` URLs do not have a
    /// guaranteed-parseable owner/repo/domain triple, and `Path` /
    /// `Indirect` have no identity at all.
    ///
    /// Honours the `?host=` override (matching Nix's per-scheme host
    /// resolution); falls back to the platform's canonical domain when
    /// no override is set. The `SourceHut` canonical default stays
    /// `git.sr.ht` rather than the apex `sr.ht`, which does not serve
    /// git over HTTPS.
    pub fn forge_identity(&self) -> Option<ForgeIdentity> {
        match self.kind() {
            FlakeRefType::GitForge(forge) => {
                let canonical = match forge.platform {
                    GitForgePlatform::GitHub => "github.com",
                    GitForgePlatform::GitLab => "gitlab.com",
                    GitForgePlatform::SourceHut => "git.sr.ht",
                };
                let domain = self
                    .params
                    .host_value()
                    .map_or_else(|| canonical.to_string(), str::to_owned);
                Some(ForgeIdentity {
                    platform: forge.platform.clone(),
                    owner: forge.owner.clone(),
                    repo: forge.repo.clone(),
                    domain,
                })
            }
            _ => None,
        }
    }

    /// The `ref_` (branch or tag name) for kinds that carry one, borrowed.
    pub fn ref_(&self) -> Option<&str> {
        match self.kind() {
            FlakeRefType::GitForge(GitForge { ref_, .. }) | FlakeRefType::Indirect { ref_, .. } => {
                ref_.as_deref()
            }
            FlakeRefType::Resource(res) => res.ref_.as_deref(),
            FlakeRefType::Path { .. } => None,
        }
    }

    /// The `rev` (40-hex commit hash) for kinds that carry one, borrowed.
    pub fn rev(&self) -> Option<&str> {
        match self.kind() {
            FlakeRefType::GitForge(GitForge { rev, .. })
            | FlakeRefType::Indirect { rev, .. }
            | FlakeRefType::Path { rev, .. } => rev.as_deref(),
            FlakeRefType::Resource(res) => res.rev.as_deref(),
        }
    }

    /// Whichever of `rev` / `ref_` is set, preferring `rev` when pinned;
    /// the typical "what does this ref resolve to?" answer. Returns
    /// borrowed `&str`; callers wanting an owned `String` use
    /// `.map(str::to_owned)`.
    pub fn ref_or_rev(&self) -> Option<&str> {
        if self.is_pinned_to_rev() {
            self.rev()
        } else {
            self.ref_().or_else(|| self.rev())
        }
    }

    /// Discriminates which of `ref_` / `rev` are populated; useful when the
    /// caller needs to switch on the four-way combination without writing
    /// nested `Option` matches.
    pub fn ref_kind(&self) -> RefKind {
        match (self.ref_().is_some(), self.rev().is_some()) {
            (false, false) => RefKind::None,
            (true, false) => RefKind::Ref,
            (false, true) => RefKind::Rev,
            (true, true) => RefKind::Both,
        }
    }

    /// `true` when the kind has a `rev` set (canonical Nix's "pinned to a
    /// commit"). False for refs-by-name and for kinds that don't carry
    /// rev at all.
    pub fn is_pinned_to_rev(&self) -> bool {
        matches!(
            self.kind(),
            FlakeRefType::GitForge(GitForge { rev: Some(_), .. })
                | FlakeRefType::Indirect { rev: Some(_), .. }
                | FlakeRefType::Path { rev: Some(_), .. }
        ) || matches!(
            self.kind(),
            FlakeRefType::Resource(res) if res.rev.is_some()
        )
    }

    /// Where the kind's ref/rev is rendered. Reads the kind's [`RefLocation`]
    /// directly; `Path` is fixed at `QueryParameter` because its rev has no
    /// path-component spelling in Nix's grammar.
    pub fn ref_source_location(&self) -> RefLocation {
        match self.kind() {
            FlakeRefType::GitForge(forge) => forge.location,
            FlakeRefType::Indirect { location, .. } => *location,
            FlakeRefType::Resource(res) => res.ref_location,
            FlakeRefType::Path { .. } => RefLocation::QueryParameter,
        }
    }

    /// Read-only access to the query-string parameters.
    pub fn params(&self) -> &LocationParameters {
        &self.params
    }

    /// Trailing `#fragment` retained verbatim. Nix uses fragments to select
    /// an attribute path inside a flake (e.g. `github:nixos/nixpkgs#hello`);
    /// the raw string is preserved without interpretation.
    pub fn fragment(&self) -> Option<&str> {
        self.fragment.as_deref()
    }

    /// Write `new_ref` into the kind's typed `ref_` slot. Silently no-op for
    /// kinds that do not carry ref/rev. For [`FlakeRefType::Resource`],
    /// writing a non-`None` value also flips `ref_location` to
    /// [`RefLocation::QueryParameter`]; Resource has no path-component
    /// ref/rev representation, so leaving it as `PathComponent` would cause
    /// `Display` to drop the value silently.
    pub fn set_ref(&mut self, new_ref: Option<String>) {
        let writing_some = new_ref.is_some();
        self.kind_mut().set_ref(new_ref);
        if writing_some && matches!(self.kind(), FlakeRefType::Resource(_)) {
            self.kind_mut()
                .set_ref_location(RefLocation::QueryParameter);
        }
    }

    /// Write `new_rev` into the kind's typed `rev` slot. See [`Self::set_ref`]
    /// for the Resource `ref_location` flip rationale.
    pub fn set_rev(&mut self, new_rev: Option<String>) {
        let writing_some = new_rev.is_some();
        self.kind_mut().set_rev(new_rev);
        if writing_some && matches!(self.kind(), FlakeRefType::Resource(_)) {
            self.kind_mut()
                .set_ref_location(RefLocation::QueryParameter);
        }
    }

    /// Write `fragment` (the `#suffix`) into the typed slot.
    pub fn set_fragment(&mut self, fragment: Option<String>) {
        self.fragment = fragment;
    }

    /// Set [`RefLocation`] on the kind's `ref_location` slot. Renamed from
    /// `set_location`, which was easy to misread as "modify the URL
    /// location string"; this method writes the routing tag that controls
    /// whether ref/rev render as `?ref=` or as a path component.
    pub fn set_ref_location(&mut self, loc: RefLocation) {
        self.kind_mut().set_ref_location(loc);
    }

    /// Typed mutator for the `dir` query parameter.
    pub fn set_dir(&mut self, dir: Option<String>) {
        self.params.set_dir(dir);
    }

    /// Typed mutator for the `host` query parameter.
    pub fn set_host(&mut self, host: Option<String>) {
        self.params.set_host(host);
    }

    /// Typed mutator for the `shallow` query parameter. `true` enables a
    /// shallow clone (mapped to `?shallow=1` on Display); `false` writes
    /// `?shallow=0`. Pass through [`LocationParameters::set_shallow`] to
    /// clear the slot entirely.
    pub fn set_shallow(&mut self, shallow: bool) {
        self.params.set_shallow(Some(shallow));
    }

    /// Typed mutator for the `submodules` query parameter; behaves like
    /// [`Self::set_shallow`].
    pub fn set_submodules(&mut self, submodules: bool) {
        self.params.set_submodules(Some(submodules));
    }

    /// Typed mutator for the `narHash` query parameter.
    pub fn set_nar_hash(&mut self, hash: Option<String>) {
        self.params.set_nar_hash(hash);
    }

    /// Typed mutator for the `lastModified` query parameter.
    pub fn set_last_modified(&mut self, ts: Option<String>) {
        self.params.set_last_modified(ts);
    }

    /// Typed mutator for the `revCount` query parameter.
    pub fn set_rev_count(&mut self, count: Option<String>) {
        self.params.set_rev_count(count);
    }

    /// Internal helper: replace the entire [`LocationParameters`] block.
    /// Kept `pub(crate)` because batch replacement is an initialisation
    /// pattern (the parser builds a fresh `LocationParameters` then
    /// installs it); public consumers should reach for the typed
    /// `set_*` mutators instead.
    pub(crate) fn replace_params(&mut self, params: LocationParameters) {
        *self.params = params;
    }

    /// Consuming builder variant of [`Self::set_ref`]. Silently no-ops
    /// on kinds without a `ref_` slot ([`FlakeRefType::Path`]); see
    /// [`Self::try_with_ref`] for the loud opt-in alternative that
    /// surfaces a typed error instead.
    pub fn with_ref(mut self, r: Option<String>) -> Self {
        self.set_ref(r);
        self
    }

    /// Consuming builder variant of [`Self::set_rev`]. See
    /// [`Self::try_with_rev`] for the fallible variant kept for API
    /// symmetry with [`Self::try_with_ref`].
    pub fn with_rev(mut self, r: Option<String>) -> Self {
        self.set_rev(r);
        self
    }

    /// Fallible variant of [`Self::with_ref`]: returns
    /// [`NixUriError::Unsupported`] when the kind cannot carry a ref per
    /// Nix's per-scheme attribute rules.
    ///
    /// Surfaces [`UnsupportedReason::Field`] for kinds outside the
    /// ref-bearing set ([`FlakeRefType::Path`], [`ResourceType::File`]
    /// and [`ResourceType::Tarball`]). Setting a ref on these kinds via
    /// [`Self::with_ref`] is a silent no-op (Path) or renders a string
    /// the parser would reject (File/Tarball, breaking the round-trip
    /// invariant); `try_with_ref` lets callers diagnose the mismatch at
    /// the call site.
    ///
    /// `try_with_ref(None)` is always `Ok`: clearing has no Nix-level
    /// implications.
    pub fn try_with_ref(self, new_ref: Option<String>) -> Result<Self, NixUriError> {
        if new_ref.is_some() && !self.kind().allows_ref() {
            return Err(NixUriError::Unsupported(UnsupportedReason::Field {
                field: "ref".into(),
                only_supported_by: "github, gitlab, sourcehut, flake (indirect), git+, hg+".into(),
            }));
        }
        Ok(self.with_ref(new_ref))
    }

    /// Fallible variant of [`Self::with_rev`]. All current kinds permit
    /// rev per Nix's per-scheme attribute rules, so this method always
    /// returns `Ok`. It exists for API symmetry with
    /// [`Self::try_with_ref`] so callers can write the same
    /// fallible-builder shape regardless of which slot they are
    /// writing.
    pub fn try_with_rev(self, new_rev: Option<String>) -> Result<Self, NixUriError> {
        Ok(self.with_rev(new_rev))
    }

    /// Consuming builder variant of [`Self::set_fragment`].
    pub fn with_fragment(mut self, fragment: Option<String>) -> Self {
        self.set_fragment(fragment);
        self
    }

    /// Consuming builder that sets the kind in a chain.
    pub fn with_kind(mut self, kind: FlakeRefType) -> Self {
        *self.kind_mut() = kind;
        self
    }

    /// Consuming builder that installs a fresh [`LocationParameters`] block.
    /// For incremental edits, use the typed `set_*` mutators instead.
    pub fn with_params(mut self, params: LocationParameters) -> Self {
        self.params = Box::new(params);
        self
    }

    /// Clear the `rev` slot (the "pin"); leaves `ref_` alone. Useful when
    /// the caller wants the named branch/tag rather than a specific
    /// commit.
    pub fn without_pin(mut self) -> Self {
        self.set_rev(None);
        self
    }

    /// Pin to a specific commit, clearing any pre-existing `ref_` first.
    ///
    /// `with_rev(Some(rev))` writes only the `rev` slot, leaving any
    /// `ref_` already on the kind in place. The `GitForge` Display arm
    /// renders `ref_.or(rev)` when [`RefLocation::PathComponent`] is
    /// active, so a `ref_`-bearing forge URL with a freshly written
    /// `rev` round-trips back to the named ref and silently drops the
    /// pinned commit. `pin_to_rev` is the atomic builder that performs
    /// the clear-ref-then-set-rev sequence callers want.
    ///
    /// For kinds without a `ref_` slot ([`FlakeRefType::Path`]), this
    /// only writes the `rev`; there is no ref to clear.
    pub fn pin_to_rev(mut self, rev: String) -> Self {
        self.set_ref(None);
        self.set_rev(Some(rev));
        self
    }

    /// Consume and render to a `String`; the same output as `to_string`
    /// but without an intermediate clone when the caller already owns
    /// `self`.
    pub fn into_uri(self) -> String {
        self.to_string()
    }

    /// Canonical wire form of this `FlakeRef`, matching the URL Nix
    /// would emit for the same input.
    ///
    /// Pairs with [`Display`] (and the equivalent [`Self::into_uri`]),
    /// which preserves the input verbatim for byte-stable round-trips.
    /// `to_canonical_string` instead emits the form Nix would produce,
    /// normalising every shape that Nix collapses on the way out:
    ///
    /// - `GitForge` (`github:` / `gitlab:` / `sourcehut:`): renders as
    ///   `<scheme>:<owner>/<repo>[/<ref-or-rev>]` regardless of where
    ///   the parsed value came from. `rev` wins over `ref_` when both
    ///   are populated, matching Nix's behaviour (Nix asserts that ref
    ///   and rev are never both set on a git-archive URL, so the
    ///   both-set case is nonsensical for consumers). The query carries
    ///   only `host` and `narHash` when set.
    /// - `Resource(Git)`: emits `ref` and `rev` always when set, and
    ///   the typed booleans (`shallow`, `lfs`, `submodules`,
    ///   `exportIgnore`, `verifyCommit`) only for the truthy branch.
    ///   `allRefs` is never emitted because Nix's git scheme does not
    ///   include it on canonical output. `narHash`, `lastModified`,
    ///   `revCount`, `dir`, `host`, and arbitrary keys are dropped.
    /// - `Resource(Mercurial)`: emits only `ref` and `rev`.
    /// - `Indirect`, `Path`, `Resource(File)`, `Resource(Tarball)`:
    ///   delegated to `Display`; their existing form already matches
    ///   Nix byte-for-byte.
    ///
    /// Use this when handing a string to a Nix consumer that expects
    /// the canonical spelling; reach for `Display` / [`Self::into_uri`]
    /// when round-tripping a user-supplied URL byte-for-byte.
    pub fn to_canonical_string(&self) -> String {
        use std::fmt::Write;

        let mut out = String::new();

        match self.kind() {
            FlakeRefType::GitForge(forge) => {
                let owner_out = encoding::encode_path_segment(&forge.owner);
                write!(&mut out, "{}:{}/{}", forge.platform, owner_out, forge.repo).unwrap();
                // ref/rev always render in the path tail. `rev` wins
                // when both happen to be populated (Nix asserts they
                // cannot coexist on a git-archive URL; we still pick a
                // deterministic answer rather than silently dropping
                // one).
                if let Some(value) = forge.rev.as_deref().or(forge.ref_.as_deref()) {
                    write!(&mut out, "/{value}").unwrap();
                }
                let mut entries: Vec<(&str, &str)> = Vec::new();
                if let Some(host) = self.params.host_value() {
                    entries.push(("host", host));
                }
                if let Some(nar) = self.params.nar_hash_value() {
                    entries.push(("narHash", nar));
                }
                entries.sort_by(|a, b| a.0.cmp(b.0));
                write_canonical_query(&mut out, &entries);
            }
            FlakeRefType::Resource(res) if matches!(res.res_type, ResourceType::Git) => {
                write_resource_base(&mut out, res);
                let mut entries: Vec<(&str, &str)> = Vec::new();
                if let Some(r) = res.ref_.as_deref() {
                    entries.push(("ref", r));
                }
                if let Some(v) = res.rev.as_deref() {
                    entries.push(("rev", v));
                }
                if self.params.shallow_truthy() {
                    entries.push(("shallow", "1"));
                }
                if self.params.lfs == Some(true) {
                    entries.push(("lfs", "1"));
                }
                if self.params.submodules_truthy() {
                    entries.push(("submodules", "1"));
                }
                if self.params.export_ignore == Some(true) {
                    entries.push(("exportIgnore", "1"));
                }
                if self.params.verify_commit == Some(true) {
                    entries.push(("verifyCommit", "1"));
                }
                if let Some(kt) = self.params.keytype.as_deref() {
                    entries.push(("keytype", kt));
                }
                if let Some(pk) = self.params.public_key.as_deref() {
                    entries.push(("publicKey", pk));
                }
                if let Some(pks) = self.params.public_keys.as_deref() {
                    entries.push(("publicKeys", pks));
                }
                entries.sort_by(|a, b| a.0.cmp(b.0));
                write_canonical_query(&mut out, &entries);
            }
            FlakeRefType::Resource(res) if matches!(res.res_type, ResourceType::Mercurial) => {
                write_resource_base(&mut out, res);
                let mut entries: Vec<(&str, &str)> = Vec::new();
                if let Some(r) = res.ref_.as_deref() {
                    entries.push(("ref", r));
                }
                if let Some(v) = res.rev.as_deref() {
                    entries.push(("rev", v));
                }
                entries.sort_by(|a, b| a.0.cmp(b.0));
                write_canonical_query(&mut out, &entries);
            }
            _ => {
                // Indirect, Path, Resource(File), Resource(Tarball):
                // the existing Display form already matches Nix's
                // canonical output byte-for-byte.
                return self.to_string();
            }
        }

        if let Some(fragment) = &self.fragment {
            write!(&mut out, "#{}", encoding::encode_fragment(fragment)).unwrap();
        }
        out
    }
}

fn write_resource_base(out: &mut String, res: &ResourceUrl) {
    use std::fmt::Write;
    // Git and Mercurial canonical forms keep the `<res_type>+` prefix
    // verbatim (no Tarball/File-style stripping); they always carry a
    // resource scheme tag.
    write!(out, "{}", res.res_type).unwrap();
    if let Some(transport) = &res.transport_type {
        write!(out, "+{}", transport).unwrap();
    }
    write!(out, "://{}", res.location).unwrap();
}

fn write_canonical_query(out: &mut String, entries: &[(&str, &str)]) {
    use std::fmt::Write;
    if entries.is_empty() {
        return;
    }
    out.push('?');
    for (i, (key, value)) in entries.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        write!(
            out,
            "{key}={value}",
            key = encoding::encode_query(key),
            value = encoding::encode_query(value)
        )
        .unwrap();
    }
}

impl Display for FlakeRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.kind())?;

        // Collect every query key (typed param slots, the arbitrary bag, and
        // the kind's ref/rev when they live in the query) into one list and
        // emit it sorted by key. Nix emits query keys in alphabetical
        // order; matching that lets a Display string compare
        // byte-for-byte against a Nix-emitted form.
        let mut entries: Vec<(&str, &str)> = self.params.entries();
        if matches!(self.ref_source_location(), RefLocation::QueryParameter) {
            // Resource only supports the query-parameter form (Nix's
            // git/hg schemes have no path-component ref/rev shape), so
            // it always emits here when ref/rev are set.
            // Path is fixed at `QueryParameter` and has no ref slot;
            // `ref_or_rev` returns `(None, rev)` for it.
            let (ref_, rev) = match self.kind() {
                FlakeRefType::GitForge(GitForge { ref_, rev, .. })
                | FlakeRefType::Indirect { ref_, rev, .. } => (ref_.as_deref(), rev.as_deref()),
                FlakeRefType::Resource(res) => (res.ref_.as_deref(), res.rev.as_deref()),
                FlakeRefType::Path { rev, .. } => (None, rev.as_deref()),
            };
            if let Some(r) = ref_ {
                entries.push(("ref", r));
            }
            if let Some(v) = rev {
                entries.push(("rev", v));
            }
        }
        entries.sort_by(|a, b| a.0.cmp(b.0));
        if !entries.is_empty() {
            write!(f, "?")?;
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
        }
        if let Some(fragment) = &self.fragment {
            write!(f, "#{}", encoding::encode_fragment(fragment))?;
        }
        Ok(())
    }
}

impl TryFrom<&str> for FlakeRef {
    type Error = NixUriError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        use crate::parser::parse_nix_uri;
        parse_nix_uri(value)
    }
}

impl std::str::FromStr for FlakeRef {
    type Err = NixUriError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use crate::parser::parse_nix_uri;
        parse_nix_uri(s)
    }
}

#[cfg(test)]
mod tests {

    use cool_asserts::assert_matches;
    use resource_url::{ResourceType, ResourceUrl};
    use winnow::Parser;

    use super::*;
    use crate::{
        NixUriResult,
        parser::{parse_nix_uri, parse_params, route_location_params},
    };

    #[test]
    fn parse_simple_uri() {
        let uri = "github:nixos/nixpkgs";
        let expected = FlakeRef::default().with_kind(FlakeRefType::GitForge(GitForge {
            platform: GitForgePlatform::GitHub,
            owner: "nixos".into(),
            repo: "nixpkgs".into(),
            ref_: None,
            rev: None,
            location: RefLocation::PathComponent,
        }));

        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn parse_simple_uri_parsed() {
        let uri = "github:zellij-org/zellij";
        let expected = FlakeRef::default().with_kind(FlakeRefType::GitForge(GitForge {
            platform: GitForgePlatform::GitHub,
            owner: "zellij-org".into(),
            repo: "zellij".into(),
            ref_: None,
            rev: None,
            location: RefLocation::PathComponent,
        }));

        let parsed: FlakeRef = uri.parse().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn parse_simple_uri_no_params() {
        let uri = "github:zellij-org/zellij";
        let parsed = parse_params.parse_peek(uri).unwrap().1;
        assert_eq!(("github:zellij-org/zellij", None), parsed);
    }

    #[test]
    fn parse_simple_uri_attr_with_params() {
        let uri = "github:zellij-org/zellij?dir=assets";
        let mut location_params = LocationParameters::default();
        location_params.dir(Some("assets".into()));
        let (head, raw_values) = parse_params.parse_peek(uri).unwrap().1;
        assert_eq!("github:zellij-org/zellij", head);
        let (params, ref_rev) = route_location_params(raw_values.unwrap()).unwrap();
        assert_eq!(location_params, params);
        assert!(ref_rev.r#ref.is_none() && ref_rev.rev.is_none());
    }

    #[test]
    fn parse_simple_uri_ref() {
        let uri = "github:zellij-org/zellij?ref=main";
        let flake_ref = FlakeRef::default().with_kind(FlakeRefType::GitForge(GitForge {
            platform: GitForgePlatform::GitHub,
            owner: "zellij-org".into(),
            repo: "zellij".into(),
            ref_: Some("main".into()),
            rev: None,
            location: RefLocation::QueryParameter,
        }));

        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }

    #[test]
    fn parse_simple_uri_rev() {
        let uri = "github:zellij-org/zellij?rev=b2df4e4e80e04cbb33a350f87717f4bd6140d298";
        let flake_ref = FlakeRef::default().with_kind(FlakeRefType::GitForge(GitForge {
            platform: GitForgePlatform::GitHub,
            owner: "zellij-org".into(),
            repo: "zellij".into(),
            ref_: None,
            rev: Some("b2df4e4e80e04cbb33a350f87717f4bd6140d298".into()),
            location: RefLocation::QueryParameter,
        }));

        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }

    #[test]
    fn parse_simple_uri_ref_or_rev() {
        let uri = "github:zellij-org/zellij/main";
        let flake_ref = FlakeRef::default().with_kind(FlakeRefType::GitForge(GitForge {
            platform: GitForgePlatform::GitHub,
            owner: "zellij-org".into(),
            repo: "zellij".into(),
            ref_: Some("main".into()),
            rev: None,
            location: RefLocation::PathComponent,
        }));

        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }

    #[test]
    fn parse_simple_uri_ref_or_rev_attr() {
        let uri = "github:zellij-org/zellij/main?dir=assets";
        let mut params = LocationParameters::default();
        params.dir(Some("assets".into()));
        let flake_ref = FlakeRef::default()
            .with_kind(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_: Some("main".into()),
                rev: None,
                location: RefLocation::PathComponent,
            }))
            .with_params(params);

        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }

    #[test]
    fn parse_simple_uri_attr() {
        let uri = "github:zellij-org/zellij?dir=assets";
        let mut params = LocationParameters::default();
        params.dir(Some("assets".into()));
        let flake_ref = FlakeRef::default()
            .with_kind(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_: None,
                rev: None,
                location: RefLocation::PathComponent,
            }))
            .with_params(params);

        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_simple_uri_attr_nom_alt() {
        let uri = "github:zellij-org/zellij/?dir=assets";
        let mut params = LocationParameters::default();
        params.dir(Some("assets".into()));
        let flake_ref = FlakeRef::default()
            .with_kind(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_: None,
                rev: None,
                location: RefLocation::PathComponent,
            }))
            .with_params(params);

        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_simple_uri_params_nom_alt() {
        let uri = "github:zellij-org/zellij/?dir=assets&narHash=fakeHash256";
        let mut params = LocationParameters::default();
        params.dir(Some("assets".into()));
        params.nar_hash(Some("fakeHash256".into()));
        let flake_ref = FlakeRef::default()
            .with_kind(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_: None,
                rev: None,
                location: RefLocation::PathComponent,
            }))
            .with_params(params);

        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }

    #[test]
    fn parse_simple_path_nom() {
        let uri = "path:/home/kenji/.config/dotfiles/";
        let flake_ref = FlakeRef::default().with_kind(FlakeRefType::Path {
            path: "/home/kenji/.config/dotfiles/".into(),
            rev: None,
        });

        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed, "{}", uri);
    }

    #[test]
    fn parse_simple_path_params_nom() {
        let uri = "path:/home/kenji/.config/dotfiles/?dir=assets";
        let mut params = LocationParameters::default();
        params.dir(Some("assets".into()));
        let flake_ref = FlakeRef::default()
            .with_kind(FlakeRefType::Path {
                path: "/home/kenji/.config/dotfiles/".into(),
                rev: None,
            })
            .with_params(params);

        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed, "{}", uri);
    }

    #[test]
    fn parse_gitlab_simple() {
        let uri = "gitlab:veloren/veloren";
        let flake_ref = FlakeRef::default().with_kind(FlakeRefType::GitForge(GitForge {
            platform: GitForgePlatform::GitLab,
            owner: "veloren".into(),
            repo: "veloren".into(),
            ref_: None,
            rev: None,
            location: RefLocation::PathComponent,
        }));

        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }

    #[test]
    fn parse_gitlab_simple_ref_or_rev() {
        let uri = "gitlab:veloren/veloren/master";
        let flake_ref = FlakeRef::default().with_kind(FlakeRefType::GitForge(GitForge {
            platform: GitForgePlatform::GitLab,
            owner: "veloren".into(),
            repo: "veloren".into(),
            ref_: Some("master".into()),
            rev: None,
            location: RefLocation::PathComponent,
        }));

        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }

    #[test]
    fn parse_gitlab_simple_ref_or_rev_alt() {
        let uri = "gitlab:veloren/veloren/19742bb9300fb0be9fdc92f30766c95230a8a371";
        let flake_ref = FlakeRef::default().with_kind(FlakeRefType::GitForge(GitForge {
            platform: GitForgePlatform::GitLab,
            owner: "veloren".into(),
            repo: "veloren".into(),
            ref_: None,
            rev: Some("19742bb9300fb0be9fdc92f30766c95230a8a371".into()),
            location: RefLocation::PathComponent,
        }));

        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }

    #[test]
    fn parse_gitlab_nested_subgroup() {
        let uri = "gitlab:veloren%2Fdev/rfcs";
        let parsed = parse_nix_uri(uri).unwrap();
        let flake_ref = FlakeRef::default().with_kind(FlakeRefType::GitForge(GitForge {
            platform: GitForgePlatform::GitLab,
            owner: "veloren/dev".into(),
            repo: "rfcs".into(),
            ref_: None,
            rev: None,
            location: RefLocation::PathComponent,
        }));
        assert_eq!(flake_ref, parsed);
        // Display re-encodes the subgroup `/` so the wire form is byte-stable.
        assert_eq!(parsed.to_string(), uri);
    }

    #[test]
    fn parse_gitlab_simple_host_param() {
        let uri = "gitlab:openldap/openldap?host=git.openldap.org";
        let mut params = LocationParameters::default();
        params.host(Some("git.openldap.org".into()));
        let flake_ref = FlakeRef::default()
            .with_kind(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitLab,
                owner: "openldap".into(),
                repo: "openldap".into(),
                ref_: None,
                rev: None,
                location: RefLocation::PathComponent,
            }))
            .with_params(params);

        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }

    #[test]
    fn parse_git_and_https_simple() {
        let uri = "git+https://git.somehost.tld/user/path";
        let expected = FlakeRef::default().with_kind(FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Git,
            location: "git.somehost.tld/user/path".into(),
            transport_type: Some(TransportLayer::Https),
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        }));

        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn parse_git_and_https_params() {
        let uri = "git+https://git.somehost.tld/user/path?ref=branch&rev=fdc8ef970de2b4634e1b3dca296e1ed918459a9e";
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(parsed.to_string(), uri);
    }

    #[test]
    fn parse_git_and_file_params() {
        let uri = "git+file:///nix/nixpkgs?ref=upstream/nixpkgs-unstable";
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(parsed.to_string(), uri);
    }

    #[test]
    fn parse_git_and_file_simple() {
        let uri = "git+file:///nix/nixpkgs";
        let expected = FlakeRef::default().with_kind(FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Git,
            location: "/nix/nixpkgs".into(),
            transport_type: Some(TransportLayer::File),
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        }));

        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn parse_git_and_file_branch_query_routes_to_arbitrary() {
        // Nix's git scheme has no `branch` key; the branch slot is
        // `ref`. The parser absorbs the unrecognised key into the
        // arbitrary bag rather than rejecting, matching Nix's
        // permissive `parseFlakeRef`.
        let uri = "git+file:///home/user/forked-flake?branch=feat/myNewFeature";
        let parsed: FlakeRef = uri.parse().expect("unrecognised key must parse");
        assert!(
            parsed
                .params()
                .entries()
                .iter()
                .any(|(k, v)| *k == "branch" && *v == "feat/myNewFeature"),
            "branch=feat/myNewFeature must land in arbitrary",
        );
    }

    #[test]
    fn parse_github_simple_tag_non_alphabetic_params() {
        let uri = "github:smunix/MyST-Parser?ref=fix.hls-docutils";
        let expected = FlakeRef::default().with_kind(FlakeRefType::GitForge(GitForge {
            platform: GitForgePlatform::GitHub,
            owner: "smunix".into(),
            repo: "MyST-Parser".into(),
            ref_: Some("fix.hls-docutils".to_owned()),
            rev: None,
            location: RefLocation::QueryParameter,
        }));

        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn parse_github_simple_tag() {
        let uri = "github:cachix/devenv/v0.5";
        let expected = FlakeRef::default().with_kind(FlakeRefType::GitForge(GitForge {
            platform: GitForgePlatform::GitHub,
            owner: "cachix".into(),
            repo: "devenv".into(),
            ref_: Some("v0.5".into()),
            rev: None,
            location: RefLocation::PathComponent,
        }));

        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn parse_gitlab_with_host_params_alt() {
        let uri = "gitlab:fpottier/menhir/20201216?host=gitlab.inria.fr";
        let mut params = LocationParameters::default();
        params.set_host(Some("gitlab.inria.fr".into()));
        let expected = FlakeRef::default()
            .with_kind(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitLab,
                owner: "fpottier".to_owned(),
                repo: "menhir".to_owned(),
                ref_: Some("20201216".to_owned()),
                rev: None,
                location: RefLocation::PathComponent,
            }))
            .with_params(params);

        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn parse_git_and_https_params_submodules() {
        let uri = "git+https://www.github.com/ocaml/ocaml-lsp?submodules=1";
        let mut params = LocationParameters::default();
        params.set_submodules(Some(true));
        let expected = FlakeRef::default()
            .with_kind(FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::Git,
                location: "www.github.com/ocaml/ocaml-lsp".to_owned(),
                transport_type: Some(TransportLayer::Https),
                ref_: None,
                rev: None,
                ref_location: RefLocation::PathComponent,
            }))
            .with_params(params);

        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn parse_marcurial_and_https_simpe_uri() {
        let uri = "hg+https://www.github.com/ocaml/ocaml-lsp";
        let expected = FlakeRef::default().with_kind(FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::Mercurial,
            location: "www.github.com/ocaml/ocaml-lsp".to_owned(),
            transport_type: Some(TransportLayer::Https),
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        }));

        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    #[should_panic(expected = "Unsupported(UriType { ty: \"gt+https\" })")]
    fn parse_git_and_https_params_submodules_wrong_type() {
        let uri = "gt+https://www.github.com/ocaml/ocaml-lsp?submodules=1";
        let mut params = LocationParameters::default();
        params.set_submodules(Some(true));
        let expected = FlakeRef::default()
            .with_kind(FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::Git,
                location: "www.github.com/ocaml/ocaml-lsp".to_owned(),
                transport_type: Some(TransportLayer::Https),
                ref_: None,
                rev: None,
                ref_location: RefLocation::PathComponent,
            }))
            .with_params(params);

        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    // TODO: https://github.com/a-kenji/nix-uri/issues/157
    #[test]
    fn parse_git_and_file_shallow() {
        let uri = "git+file:/path/to/repo?shallow=1";
        let mut params = LocationParameters::default();
        params.set_shallow(Some(true));
        let expected = FlakeRef::default()
            .with_kind(FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::Git,
                location: "/path/to/repo".to_owned(),
                transport_type: Some(TransportLayer::File),
                ref_: None,
                rev: None,
                ref_location: RefLocation::PathComponent,
            }))
            .with_params(params);

        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn parse_simple_path_uri_indirect() {
        let uri = "path:../.";
        let expected = FlakeRef::default().with_kind(FlakeRefType::Path {
            path: "../.".to_owned(),
            rev: None,
        });
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn parse_path_uri_empty_body_rejected() {
        // The path: branch admits `path:.` and `path:../.`, so it does
        // not gate on `Path::is_absolute()`. An explicit guard on
        // empty / whitespace bodies keeps `path:` (and friends) from
        // parsing to `FlakeRefType::Path { path: "" }` and round-tripping
        // back into parse_nix_uri's trim-empty guard.
        for uri in ["path:", "path: ", "path:  "] {
            let result: Result<FlakeRef, _> = uri.try_into();
            assert!(
                matches!(result, Err(NixUriError::InvalidUrl(_))),
                "expected InvalidUrl for {uri:?}, got {result:?}"
            );
        }
    }

    #[test]
    fn parse_simple_path_uri_indirect_local() {
        let uri = "path:.";
        let expected = FlakeRef::default().with_kind(FlakeRefType::Path {
            path: ".".to_owned(),
            rev: None,
        });
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn parse_simple_uri_sourcehut() {
        let uri = "sourcehut:~misterio/nix-colors";
        let expected = FlakeRef::default().with_kind(FlakeRefType::GitForge(GitForge {
            platform: GitForgePlatform::SourceHut,
            owner: "~misterio".to_owned(),
            repo: "nix-colors".to_owned(),
            ref_: None,
            rev: None,
            location: RefLocation::PathComponent,
        }));

        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn parse_simple_uri_sourcehut_rev() {
        let uri = "sourcehut:~misterio/nix-colors/main";
        let expected = FlakeRef::default().with_kind(FlakeRefType::GitForge(GitForge {
            platform: GitForgePlatform::SourceHut,
            owner: "~misterio".to_owned(),
            repo: "nix-colors".to_owned(),
            ref_: Some("main".to_owned()),
            rev: None,
            location: RefLocation::PathComponent,
        }));

        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn parse_simple_uri_sourcehut_host_param() {
        let uri = "sourcehut:~misterio/nix-colors?host=git.example.org";
        let mut params = LocationParameters::default();
        params.set_host(Some("git.example.org".into()));
        let expected = FlakeRef::default()
            .with_kind(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::SourceHut,
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_: None,
                rev: None,
                location: RefLocation::PathComponent,
            }))
            .with_params(params);

        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn parse_simple_uri_sourcehut_ref() {
        let uri = "sourcehut:~misterio/nix-colors/182b4b8709b8ffe4e9774a4c5d6877bf6bb9a21c";
        let expected = FlakeRef::default().with_kind(FlakeRefType::GitForge(GitForge {
            platform: GitForgePlatform::SourceHut,
            owner: "~misterio".to_owned(),
            repo: "nix-colors".to_owned(),
            ref_: None,
            rev: Some("182b4b8709b8ffe4e9774a4c5d6877bf6bb9a21c".to_owned()),
            location: RefLocation::PathComponent,
        }));

        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn parse_simple_uri_sourcehut_ref_params() {
        let uri =
            "sourcehut:~misterio/nix-colors/21c1a380a6915d890d408e9f22203436a35bb2de?host=hg.sr.ht";
        let mut params = LocationParameters::default();
        params.set_host(Some("hg.sr.ht".into()));
        let expected = FlakeRef::default()
            .with_kind(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::SourceHut,
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_: None,
                rev: Some("21c1a380a6915d890d408e9f22203436a35bb2de".to_owned()),
                location: RefLocation::PathComponent,
            }))
            .with_params(params);

        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn display_simple_sourcehut_uri_ref_or_rev() {
        let expected = "sourcehut:~misterio/nix-colors/21c1a380a6915d890d408e9f22203436a35bb2de";
        let flake_ref = FlakeRef::default()
            .with_kind(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::SourceHut,
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_: None,
                rev: Some("21c1a380a6915d890d408e9f22203436a35bb2de".to_owned()),
                location: RefLocation::PathComponent,
            }))
            .to_string();

        assert_eq!(expected, flake_ref);
    }

    #[test]
    fn display_simple_sourcehut_uri_ref_or_rev_host_param() {
        let expected =
            "sourcehut:~misterio/nix-colors/21c1a380a6915d890d408e9f22203436a35bb2de?host=hg.sr.ht";
        let mut params = LocationParameters::default();
        params.set_host(Some("hg.sr.ht".into()));
        let flake_ref = FlakeRef::default()
            .with_kind(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::SourceHut,
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_: None,
                rev: Some("21c1a380a6915d890d408e9f22203436a35bb2de".to_owned()),
                location: RefLocation::PathComponent,
            }))
            .with_params(params)
            .to_string();

        assert_eq!(expected, flake_ref);
    }

    #[test]
    fn display_simple_github_uri_ref() {
        let expected = "github:zellij-org/zellij?ref=main";
        let flake_ref = FlakeRef::default()
            .with_kind(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_: Some("main".into()),
                rev: None,
                location: RefLocation::QueryParameter,
            }))
            .to_string();

        assert_eq!(flake_ref, expected);
    }

    #[test]
    fn display_simple_github_uri_rev() {
        let expected = "github:zellij-org/zellij?rev=b2df4e4e80e04cbb33a350f87717f4bd6140d298";
        let flake_ref = FlakeRef::default()
            .with_kind(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_: None,
                rev: Some("b2df4e4e80e04cbb33a350f87717f4bd6140d298".into()),
                location: RefLocation::QueryParameter,
            }))
            .to_string();

        assert_eq!(flake_ref, expected);
    }

    #[test]
    fn parse_simple_path_uri_indirect_absolute_without_prefix() {
        let uri = "/home/kenji/git";
        let expected = FlakeRef::default().with_kind(FlakeRefType::Path {
            path: "/home/kenji/git".to_owned(),
            rev: None,
        });

        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn parse_simple_path_uri_indirect_absolute_without_prefix_with_params() {
        let uri = "/home/kenji/git?dir=dev";
        let mut params = LocationParameters::default();
        params.set_dir(Some("dev".into()));
        let expected = FlakeRef::default()
            .with_kind(FlakeRefType::Path {
                path: "/home/kenji/git".to_owned(),
                rev: None,
            })
            .with_params(params);

        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn parse_simple_path_uri_indirect_local_without_prefix() {
        let uri = ".";
        let expected = FlakeRef::default().with_kind(FlakeRefType::Path {
            path: ".".to_owned(),
            rev: None,
        });
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn parse_wrong_git_uri_extension_type() {
        let uri = "git+(:z";
        let parsed: NixUriResult<FlakeRef> = uri.try_into();
        let parsed = parsed.unwrap_err();
        assert_matches!(
            parsed,
            NixUriError::Unsupported(UnsupportedReason::TransportLayer { ty })
                => assert_eq!("(", ty)
        );
    }

    #[test]
    fn parse_github_missing_parameter_public_surface() {
        use crate::ParseExpected;

        assert_matches!(
            parse_nix_uri("github:"),
            Err(NixUriError::Parse {
                position: 7,
                expected: ParseExpected::Label("TakeTill1"),
            })
        );
    }

    #[test]
    fn parse_github_missing_parameter_repo_public_surface() {
        use crate::ParseExpected;

        assert_matches!(
            parse_nix_uri("github:nixos/"),
            Err(NixUriError::Parse {
                position: 13,
                expected: ParseExpected::Label("TakeTill1"),
            })
        );
    }

    #[test]
    fn parse_resource_missing_separator_pins_tag_variant() {
        use crate::ParseExpected;

        assert_matches!(
            parse_nix_uri("git:x"),
            Err(NixUriError::Parse {
                position: 4,
                expected: ParseExpected::Tag("//"),
            })
        );
    }

    #[test]
    fn parse_github_starts_with_whitespace() {
        let uri = " github:nixos/nixpkgs";
        assert_matches!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::InvalidUrl(uri_match)) => assert_eq!(uri, uri_match)
        );
    }

    #[test]
    fn parse_github_ends_with_whitespace() {
        let uri = "github:nixos/nixpkgs ";
        assert_matches!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::InvalidUrl(uri_match)) => assert_eq!(uri, uri_match)
        );
    }

    #[test]
    fn parse_empty_invalid_url() {
        let uri = "";
        assert_matches!(
            uri.parse::<FlakeRef>().unwrap_err(),
            NixUriError::InvalidUrl(uri) => assert_eq!("", uri)
        );
    }

    #[test]
    fn parse_empty_trim_invalid_url() {
        let uri = "  ";
        assert_matches!(
            uri.parse::<FlakeRef>().unwrap_err(),
            NixUriError::InvalidUrl(uri_match) => assert_eq!(uri, uri_match)
        );
    }

    #[test]
    fn parse_slash_trim_invalid_url() {
        let uri = "   /   ";
        assert_matches!(
            uri.parse::<FlakeRef>().unwrap_err(),
            NixUriError::InvalidUrl(uri_match) => assert_eq!(uri, uri_match)
        );
    }

    #[test]
    fn parse_double_trim_invalid_url() {
        let uri = "   :   ";
        assert_matches!(
            uri.parse::<FlakeRef>().unwrap_err(),
            NixUriError::InvalidUrl(uri_match) => assert_eq!(uri, uri_match)
        );
    }

    #[test]
    fn indirect_display_emits_flake_prefix() {
        // Indirect Display now emits `flake:` unconditionally; the bare-input
        // form parses but Display canonicalises to the explicit prefix.
        let parsed: FlakeRef = "flake:nixpkgs/release-23.05".parse().unwrap();
        assert_eq!(parsed.to_string(), "flake:nixpkgs/release-23.05");

        let parsed: FlakeRef = "nixpkgs".parse().unwrap();
        assert_eq!(parsed.to_string(), "flake:nixpkgs");
    }

    #[test]
    fn path_display_emits_path_prefix() {
        // Same canonicalisation for Path: the `path:` prefix is always
        // emitted, even when the input form was bare.
        let parsed: FlakeRef = "path:./foo".parse().unwrap();
        assert_eq!(parsed.to_string(), "path:./foo");

        let parsed: FlakeRef = "/abs/path".parse().unwrap();
        assert_eq!(parsed.to_string(), "path:/abs/path");
    }

    #[test]
    fn indirect_explicit_three_segment_round_trip() {
        // Canonical Nix's indirect form supports id/ref/rev as three path
        // segments; round-trip preserves both.
        let uri = "flake:nixpkgs/release-23.05/abc1234567890123456789012345678901234567";
        let parsed: FlakeRef = uri.parse().unwrap();
        assert_eq!(parsed.to_string(), uri);
    }

    #[test]
    fn fragment_round_trip_github() {
        // The trailing `#fragment` is preserved on `FlakeRef.fragment`
        // and re-emitted by Display.
        let uri = "github:nixos/nixpkgs#default";
        let parsed: FlakeRef = uri.parse().unwrap();
        assert_eq!(parsed.fragment.as_deref(), Some("default"));
        assert_eq!(parsed.to_string(), uri);
    }

    #[test]
    fn fragment_round_trip_with_params() {
        let uri = "github:nixos/nixpkgs?dir=foo#bar";
        let parsed: FlakeRef = uri.parse().unwrap();
        assert_eq!(parsed.fragment.as_deref(), Some("bar"));
        assert_eq!(parsed.to_string(), uri);
    }

    /// Nix's bare-flake-id form matches `id[/ref[/rev]]` and routes it
    /// through the same indirect scheme as `flake:id/...`. So
    /// `nixos/nixpkgs` parses as
    /// `Indirect { id: "nixos", ref_: Some("nixpkgs"), .. }`.
    #[test]
    fn bare_two_segment_parses_as_indirect() {
        let parsed: FlakeRef = "nixos/nixpkgs".parse().unwrap();
        assert_eq!(
            *parsed.kind(),
            FlakeRefType::Indirect {
                id: "nixos".to_string(),
                ref_: Some("nixpkgs".to_string()),
                rev: None,
                location: RefLocation::PathComponent,
            },
        );
        assert_eq!(parsed.to_string(), "flake:nixos/nixpkgs");
    }

    /// Three-segment bare with a 40-hex final segment routes through the
    /// indirect grammar's `id/ref/rev` form, matching Nix.
    #[test]
    fn bare_three_segment_with_hex_parses_as_indirect() {
        let rev = "abc1234567890123456789012345678901234567";
        let uri = format!("nixos/nixpkgs/{rev}");
        let parsed: FlakeRef = uri.parse().unwrap();
        assert_eq!(
            *parsed.kind(),
            FlakeRefType::Indirect {
                id: "nixos".to_string(),
                ref_: Some("nixpkgs".to_string()),
                rev: Some(rev.to_string()),
                location: RefLocation::PathComponent,
            },
        );
    }

    /// Nix's bare-flake-id regex does not match four bare segments, so
    /// `nixos/nixpkgs/extra/parts` is rejected. nix-uri keeps surfacing
    /// that as `MissingScheme`.
    #[test]
    fn bare_four_segment_rejected() {
        let err = "nixos/nixpkgs/extra/parts"
            .parse::<FlakeRef>()
            .expect_err("bare four-segment must not parse");
        assert_matches!(err, NixUriError::MissingScheme { input } if input == "nixos/nixpkgs/extra/parts");
    }

    /// Nix requires the third indirect segment to be a 40-hex commit
    /// hash and throws otherwise. Without the check,
    /// `release-23.05/notahex` was silently folded into a
    /// slash-containing ref.
    #[test]
    fn flake_three_segment_non_hex_rejects() {
        let err = "flake:nixpkgs/release-23.05/notahex"
            .parse::<FlakeRef>()
            .expect_err("non-hex third segment must reject");
        assert_matches!(err, NixUriError::InvalidValue { field: "rev", .. },);
    }

    /// Nix's `flake:` URL form skips empty segments when splitting the
    /// path, so `flake:nixpkgs//main` collapses to `flake:nixpkgs/main`.
    #[test]
    fn flake_double_slash_collapses_skipempty() {
        let parsed: FlakeRef = "flake:nixpkgs//main".parse().unwrap();
        assert_eq!(
            *parsed.kind(),
            FlakeRefType::Indirect {
                id: "nixpkgs".to_string(),
                ref_: Some("main".to_string()),
                rev: None,
                location: RefLocation::PathComponent,
            },
        );

        let parsed: FlakeRef = "flake:nixpkgs///main".parse().unwrap();
        assert_eq!(
            *parsed.kind(),
            FlakeRefType::Indirect {
                id: "nixpkgs".to_string(),
                ref_: Some("main".to_string()),
                rev: None,
                location: RefLocation::PathComponent,
            },
        );
    }

    /// Bare `//host/path` (no `path:` prefix) was silently stored as
    /// `Path { path: "//host/path" }`, which Display emitted as
    /// `path://host/path` -- a string the parser then rejects on
    /// re-parse via the authority guard. Reject the malformed shape
    /// up-front instead.
    #[test]
    fn bare_double_slash_rejects() {
        let err = "//host/path"
            .parse::<FlakeRef>()
            .expect_err("bare //host/path must reject");
        assert_matches!(err, NixUriError::InvalidUrl(input) if input == "//host/path");
    }

    /// Sanity: the legitimate bare-path shapes still round-trip after
    /// the bare-`//` guard. `path:` prefix is added on Display per the
    /// canonicalisation rule pinned in `path_display_emits_path_prefix`.
    #[test]
    fn bare_legitimate_paths_round_trip() {
        for (input, displayed) in [
            ("./relative", "path:./relative"),
            ("/abs/path", "path:/abs/path"),
        ] {
            let parsed: FlakeRef = input.parse().unwrap();
            assert!(matches!(parsed.kind(), FlakeRefType::Path { .. }));
            assert_eq!(parsed.to_string(), displayed);
        }
    }

    /// Pins Nix's three-segment cap on the indirect form. Without the
    /// check the trailing segments collapse into `ref_` verbatim,
    /// producing a ref name that contains `/`.
    #[test]
    fn flake_scheme_four_segment_rejected() {
        let err = "flake:nixpkgs/main/abc/extra"
            .parse::<FlakeRef>()
            .expect_err("flake: 4+ segments must not parse");
        assert_matches!(err, NixUriError::TooManyIndirectSegments { count: 4 });
    }

    #[test]
    fn bare_single_segment_still_parses() {
        let parsed: FlakeRef = "nixpkgs".parse().unwrap();
        assert_eq!(
            *parsed.kind(),
            FlakeRefType::Indirect {
                id: "nixpkgs".to_string(),
                ref_: None,
                rev: None,
                location: RefLocation::PathComponent,
            },
        );
    }
}

#[cfg(test)]
mod ref_rev_methods {
    //! Exercises the typed ref/rev API on `FlakeRef`. The "no ref/rev" state
    //! is `ref_or_rev() == None`; there is no `RefLocation::None` variant,
    //! so `ref_source_location()` returns the kind's default `PathComponent`
    //! for kinds that simply have no value set.
    //!
    //! `Resource` kinds carry typed ref/rev slots and only support the
    //! query-parameter form; `set_ref`/`set_rev` write through and also flip
    //! `ref_location` to `QueryParameter` so the value round-trips through
    //! `Display`.
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(
        "github:nixos/nixpkgs/release-23.05",
        Some("release-23.05"),
        RefLocation::PathComponent
    )]
    #[case(
        "github:nixos/nixpkgs?ref=release-23.05",
        Some("release-23.05"),
        RefLocation::QueryParameter
    )]
    #[case(
        "github:nixos/nixpkgs?rev=b2df4e4e80e04cbb33a350f87717f4bd6140d298",
        Some("b2df4e4e80e04cbb33a350f87717f4bd6140d298"),
        RefLocation::QueryParameter
    )]
    #[case("flake:nixpkgs/unstable", Some("unstable"), RefLocation::PathComponent)]
    #[case("github:nixos/nixpkgs", None, RefLocation::PathComponent)]
    fn typed_ref_or_rev_round_trip(
        #[case] url: &str,
        #[case] expected_ref: Option<&str>,
        #[case] expected_location: RefLocation,
    ) {
        let parsed: FlakeRef = url.parse().unwrap();
        assert_eq!(
            parsed.ref_or_rev(),
            expected_ref,
            "ref_or_rev mismatch for {url}",
        );
        assert_eq!(
            parsed.ref_source_location(),
            expected_location,
            "ref_source_location mismatch for {url}",
        );
    }

    #[test]
    fn set_ref_preserves_path_component_location() {
        let url = "github:nixos/nixpkgs/release-23.05";
        let mut parsed: FlakeRef = url.parse().unwrap();

        assert_eq!(parsed.ref_source_location(), RefLocation::PathComponent);
        assert_eq!(parsed.ref_or_rev(), Some("release-23.05"));

        parsed.set_ref(Some("release-24.05".to_string()));

        assert_eq!(parsed.ref_source_location(), RefLocation::PathComponent);
        assert_eq!(parsed.ref_or_rev(), Some("release-24.05"));
        assert_eq!(parsed.to_string(), "github:nixos/nixpkgs/release-24.05");
    }

    #[test]
    fn set_ref_preserves_query_parameter_location() {
        let url = "github:nixos/nixpkgs?ref=release-23.05";
        let mut parsed: FlakeRef = url.parse().unwrap();

        assert_eq!(parsed.ref_source_location(), RefLocation::QueryParameter);
        assert_eq!(parsed.ref_or_rev(), Some("release-23.05"));

        parsed.set_ref(Some("release-24.05".to_string()));

        assert_eq!(parsed.ref_source_location(), RefLocation::QueryParameter);
        assert_eq!(parsed.ref_or_rev(), Some("release-24.05"));
        assert_eq!(parsed.to_string(), "github:nixos/nixpkgs?ref=release-24.05");
    }

    #[test]
    fn set_ref_on_resource_writes_to_typed_slot_and_flips_location() {
        // Resource kinds carry typed `ref_` / `rev` slots and only emit
        // those slots through the query string. set_ref / set_rev
        // therefore flip ref_location to QueryParameter so Display
        // preserves the form.
        let url = "git+https://github.com/nixos/nixpkgs";
        let mut parsed: FlakeRef = url.parse().unwrap();

        parsed.set_ref(Some("v1.0.0".to_string()));
        assert_eq!(parsed.ref_or_rev(), Some("v1.0.0"));
        assert_eq!(parsed.ref_source_location(), RefLocation::QueryParameter);
        match parsed.kind() {
            FlakeRefType::Resource(res) => {
                assert_eq!(res.ref_.as_deref(), Some("v1.0.0"));
            }
            other => panic!("expected Resource, got {other:?}"),
        }
        // The full round-trip works now: Display renders ?ref=...
        assert_eq!(
            parsed.to_string(),
            "git+https://github.com/nixos/nixpkgs?ref=v1.0.0",
        );
    }

    #[test]
    fn set_ref_on_github_without_existing_ref_uses_path_component() {
        let url = "github:nixos/nixpkgs";
        let mut parsed: FlakeRef = url.parse().unwrap();

        // Default (no ref present) reports PathComponent; that's where a
        // value would be rendered if set.
        assert_eq!(parsed.ref_source_location(), RefLocation::PathComponent);

        parsed.set_ref(Some("release-23.05".to_string()));

        assert_eq!(parsed.ref_source_location(), RefLocation::PathComponent);
        assert_eq!(parsed.to_string(), "github:nixos/nixpkgs/release-23.05");
    }

    #[test]
    fn set_rev_preserves_location() {
        // Path-based 40-hex rev.
        let url = "github:nixos/nixpkgs/b2df4e4e80e04cbb33a350f87717f4bd6140d298";
        let mut parsed: FlakeRef = url.parse().unwrap();

        parsed.set_rev(Some("c3ee5f5f91f15dcb44b461g98828g5ce7251e399".to_string()));
        assert_eq!(parsed.ref_source_location(), RefLocation::PathComponent);
        assert_eq!(
            parsed.to_string(),
            "github:nixos/nixpkgs/c3ee5f5f91f15dcb44b461g98828g5ce7251e399",
        );

        // Query-parameter rev.
        let url2 = "github:nixos/nixpkgs?rev=b2df4e4e80e04cbb33a350f87717f4bd6140d298";
        let mut parsed2: FlakeRef = url2.parse().unwrap();

        parsed2.set_rev(Some("c3ee5f5f91f15dcb44b461g98828g5ce7251e399".to_string()));
        assert_eq!(parsed2.ref_source_location(), RefLocation::QueryParameter);
        assert_eq!(
            parsed2.to_string(),
            "github:nixos/nixpkgs?rev=c3ee5f5f91f15dcb44b461g98828g5ce7251e399",
        );
    }

    #[test]
    fn remove_ref_clears_value_and_drops_path_segment() {
        // Path-based.
        let url = "github:nixos/nixpkgs/release-23.05";
        let mut parsed: FlakeRef = url.parse().unwrap();

        parsed.set_ref(None);
        assert_eq!(parsed.ref_or_rev(), None);
        assert_eq!(parsed.to_string(), "github:nixos/nixpkgs");

        // Query-parameter.
        let url2 = "github:nixos/nixpkgs?ref=release-23.05";
        let mut parsed2: FlakeRef = url2.parse().unwrap();

        parsed2.set_ref(None);
        assert_eq!(parsed2.ref_or_rev(), None);
        assert_eq!(parsed2.to_string(), "github:nixos/nixpkgs");
    }

    #[test]
    fn indirect_set_ref_uses_path_component() {
        let url = "flake:nixpkgs";
        let mut parsed: FlakeRef = url.parse().unwrap();

        parsed.set_ref(Some("unstable".to_string()));
        assert_eq!(parsed.ref_source_location(), RefLocation::PathComponent);
        assert_eq!(parsed.to_string(), "flake:nixpkgs/unstable");
    }

    #[test]
    fn round_trip_path_component_ref() {
        let original = "github:nixos/nixpkgs/release-23.05";
        let parsed: FlakeRef = original.parse().unwrap();
        assert_eq!(parsed.to_string(), original);
    }

    #[test]
    fn round_trip_query_parameter_ref() {
        let original = "github:nixos/nixpkgs?ref=release-23.05";
        let parsed: FlakeRef = original.parse().unwrap();
        assert_eq!(parsed.to_string(), original);
    }

    #[test]
    fn round_trip_path_component_rev() {
        let original = "github:nixos/nixpkgs/b2df4e4e80e04cbb33a350f87717f4bd6140d298";
        let parsed: FlakeRef = original.parse().unwrap();
        assert_eq!(parsed.to_string(), original);
        // The 40-hex value classified as a rev, not a ref.
        match parsed.kind() {
            FlakeRefType::GitForge(forge) => {
                assert!(forge.ref_.is_none());
                assert_eq!(
                    forge.rev.as_deref(),
                    Some("b2df4e4e80e04cbb33a350f87717f4bd6140d298"),
                );
            }
            _ => panic!("expected GitForge"),
        }
    }

    #[test]
    fn resource_set_ref_none_keeps_ref_location_when_rev_remains() {
        // Clearing one slot must not silently flip ref_location.
        // While rev is still Some, the kind's ref_location is
        // load-bearing for `Display` (it is what makes ?rev=... appear).
        let url = "git+https://github.com/owner/repo?ref=main&rev=b2df4e4e80e04cbb33a350f87717f4bd6140d298";
        let mut parsed: FlakeRef = url.parse().unwrap();
        assert_eq!(parsed.ref_source_location(), RefLocation::QueryParameter);

        parsed.set_ref(None);
        assert_eq!(parsed.ref_(), None);
        assert_eq!(
            parsed.rev(),
            Some("b2df4e4e80e04cbb33a350f87717f4bd6140d298"),
        );
        assert_eq!(
            parsed.ref_source_location(),
            RefLocation::QueryParameter,
            "clearing ref must not flip ref_location while rev is still set",
        );

        // Clearing the remaining rev now reaches the (None, None) state. The
        // location slot is informational at that point; what matters is that
        // we did not see a spurious flip on the way here.
        parsed.set_rev(None);
        assert_eq!(parsed.ref_(), None);
        assert_eq!(parsed.rev(), None);
        assert_eq!(
            parsed.ref_source_location(),
            RefLocation::QueryParameter,
            "clearing rev must not flip ref_location either",
        );
    }

    #[test]
    fn set_ref_and_rev_independently_on_gitforge() {
        let url = "github:owner/repo";
        let mut parsed: FlakeRef = url.parse().unwrap();

        parsed.set_ref(Some("main".to_string()));
        match parsed.kind() {
            FlakeRefType::GitForge(forge) => {
                assert_eq!(forge.ref_.as_deref(), Some("main"));
                assert!(forge.rev.is_none());
            }
            _ => panic!("expected GitForge"),
        }

        // Setting rev does not clear ref; the typed slots are independent.
        parsed.set_rev(Some("b2df4e4e80e04cbb33a350f87717f4bd6140d298".to_string()));
        match parsed.kind() {
            FlakeRefType::GitForge(forge) => {
                assert_eq!(forge.ref_.as_deref(), Some("main"));
                assert_eq!(
                    forge.rev.as_deref(),
                    Some("b2df4e4e80e04cbb33a350f87717f4bd6140d298"),
                );
            }
            _ => panic!("expected GitForge"),
        }
    }
}

#[cfg(test)]
mod canonical_round_trip {
    //! Round-trip property: every URI listed here parses and `Display`s back
    //! to the original string, byte-for-byte. Each case pins a distinct
    //! grammar shape (typed Resource ref/rev, Indirect 3-segment,
    //! Path/Indirect prefix normalisation, fragment retention, canonical
    //! query keys).
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case("github:nixos/nixpkgs/release-23.05")]
    #[case("github:nixos/nixpkgs?ref=release-23.05")]
    #[case("git+https://github.com/owner/repo?ref=v1.0.0")]
    #[case("flake:nixpkgs/release-23.05/abc1234567890123456789012345678901234567")]
    #[case("path:./foo")]
    #[case("github:nixos/nixpkgs#default")]
    // GitHub's URL parser narrows query keys to `ref/rev/host/narHash`;
    // the lastModified+revCount+narHash mix below rides the Git scheme
    // where all three are recognised.
    #[case("git+https://example.com/repo?lastModified=12345&narHash=sha256-abc&revCount=42")]
    fn round_trip(#[case] uri: &str) {
        let parsed: FlakeRef = uri.parse().unwrap();
        assert_eq!(parsed.to_string(), uri, "round-trip mismatch");
    }

    #[test]
    fn query_keys_emit_alphabetical_across_typed_and_arbitrary() {
        // `name` is in Nix's git-scheme attribute set but does not have
        // a typed slot on `LocationParameters`, so it lands in the
        // arbitrary bag while `dir` and `narHash` ride typed slots.
        // The Display merge sorts alphabetically across both:
        // dir < name < narHash.
        let input = "git+https://example.com/repo?narHash=sha256-x&dir=foo&name=my-flake";
        let parsed: FlakeRef = input.parse().unwrap();
        assert_eq!(
            parsed.to_string(),
            "git+https://example.com/repo?dir=foo&name=my-flake&narHash=sha256-x"
        );

        let reparsed: FlakeRef = parsed.to_string().parse().unwrap();
        assert_eq!(parsed, reparsed);
        assert_eq!(reparsed.to_string(), parsed.to_string());
    }
}

#[cfg(test)]
mod resource_prefix_strip {
    //! `tarball+` and `file+` are accepted on parse but stripped on Display.
    //! This matches Nix's canonical output for the curl-based fetcher,
    //! so a Display string handed to Nix's parser stays string-equal to
    //! what Nix would emit itself.
    use cool_asserts::assert_matches;

    use super::*;
    use crate::{ResourceType, TransportLayer, flakeref::resource_url::ResourceUrl};

    #[test]
    fn tarball_explicit_prefix_strips_on_display() {
        let parsed: FlakeRef = "tarball+https://example.com/foo.tar.gz".parse().unwrap();
        assert_eq!(parsed.to_string(), "https://example.com/foo.tar.gz");
    }

    #[test]
    fn tarball_bare_https_round_trips() {
        let input = "https://example.com/foo.tar.gz";
        let parsed: FlakeRef = input.parse().unwrap();
        assert_eq!(parsed.to_string(), input);
    }

    #[test]
    fn file_explicit_prefix_strips_on_display() {
        let parsed: FlakeRef = "file+https://example.com/data.bin".parse().unwrap();
        assert_eq!(parsed.to_string(), "https://example.com/data.bin");
    }

    #[test]
    fn file_bare_https_round_trips() {
        let input = "https://example.com/data.bin";
        let parsed: FlakeRef = input.parse().unwrap();
        assert_eq!(parsed.to_string(), input);
    }

    #[test]
    fn bare_file_with_tarball_extension_parses_as_tarball() {
        // Nix accepts a bare `file://...tar.gz` as a tarball because the
        // tarball-extension classifier matches; the file scheme rejects
        // it for the same reason. nix-uri must match that decision so
        // the round-trip from the explicit `tarball+file://` shape is
        // stable across parse -> Display -> parse.
        let parsed: FlakeRef = "file:///tmp/foo.tar.gz".parse().unwrap();
        assert_matches!(
            *parsed.kind(),
            FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::Tarball,
                transport_type: Some(TransportLayer::File),
                ..
            })
        );
    }

    #[test]
    fn bare_file_no_extension_parses_as_file() {
        // Nix's file scheme accepts a bare `file://` URL with no
        // tarball extension; the tarball scheme rejects it for the same
        // reason. nix-uri's extension-based `is_tarball` classifier
        // matches that decision.
        let parsed: FlakeRef = "file:///tmp/data.bin".parse().unwrap();
        assert_matches!(
            *parsed.kind(),
            FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::File,
                transport_type: Some(TransportLayer::File),
                ..
            })
        );
    }

    #[test]
    fn tarball_plus_file_round_trips() {
        // Parsing `tarball+file:///path/to/file.tar.gz` produces
        // `Resource(Tarball, File)`; Display strips the `tarball+`
        // prefix to `file:///path/to/file.tar.gz`; re-parse must land
        // on the same `Resource(Tarball, File)` variant so the
        // round-trip is stable.
        let input = "tarball+file:///x/y.tar.gz";
        let parsed: FlakeRef = input.parse().unwrap();
        let displayed = parsed.to_string();
        assert_eq!(displayed, "file:///x/y.tar.gz");
        let reparsed: FlakeRef = displayed.parse().unwrap();
        assert_eq!(parsed, reparsed);
        assert_eq!(reparsed.to_string(), displayed);
    }
}

#[cfg(test)]
mod accessors {
    //! Identity (owner / repo / domain / `forge_identity`) and ref/rev
    //! accessors on `FlakeRef`: the public surface that replaces
    //! triple-pattern-matches at the call site.
    use super::*;
    use rstest::rstest;

    #[test]
    fn forge_identity_for_github() {
        let parsed: FlakeRef = "github:nixos/nixpkgs".parse().unwrap();
        let id = parsed.forge_identity().unwrap();
        assert_eq!(id.platform, GitForgePlatform::GitHub);
        assert_eq!(id.owner, "nixos");
        assert_eq!(id.repo, "nixpkgs");
        assert_eq!(id.domain, "github.com");
    }

    #[test]
    fn forge_identity_for_sourcehut() {
        // Nix uses `git.sr.ht` for SourceHut clone URLs; the apex
        // `sr.ht` does not serve git over HTTPS, so a downstream that
        // hands the returned `domain` to a fetcher would 404.
        let parsed: FlakeRef = "sourcehut:~owner/repo".parse().unwrap();
        let id = parsed.forge_identity().unwrap();
        assert_eq!(id.platform, GitForgePlatform::SourceHut);
        assert_eq!(id.owner, "~owner");
        assert_eq!(id.repo, "repo");
        assert_eq!(id.domain, "git.sr.ht");
    }

    #[test]
    fn sourcehut_round_trips() {
        let uri = "sourcehut:nix-community/foo";
        let parsed: FlakeRef = uri.parse().unwrap();
        assert_eq!(parsed.to_string(), uri, "round-trip mismatch");
        assert_eq!(parsed.domain(), Some("git.sr.ht"));
    }

    #[test]
    fn gitlab_with_host_override_returns_overridden_domain() {
        // Self-hosted GitLab via `?host=` mirrors Nix's host-attr
        // resolution: the host attr overrides the canonical domain.
        // Without the override, fetch URLs target gitlab.com and
        // silently break for self-hosted instances.
        let parsed: FlakeRef = "gitlab:openldap/openldap?host=git.openldap.org"
            .parse()
            .unwrap();
        let id = parsed.forge_identity().unwrap();
        assert_eq!(id.domain, "git.openldap.org");
        assert_eq!(parsed.domain(), Some("git.openldap.org"));
    }

    #[test]
    fn github_without_host_returns_canonical_domain() {
        let parsed: FlakeRef = "github:o/r".parse().unwrap();
        let id = parsed.forge_identity().unwrap();
        assert_eq!(id.domain, "github.com");
        assert_eq!(parsed.domain(), Some("github.com"));
    }

    #[test]
    fn sourcehut_without_host_returns_git_sr_ht() {
        // SourceHut's canonical clone host is `git.sr.ht`; the apex
        // `sr.ht` does not serve git over HTTPS.
        let parsed: FlakeRef = "sourcehut:~user/repo".parse().unwrap();
        let id = parsed.forge_identity().unwrap();
        assert_eq!(id.domain, "git.sr.ht");
        assert_eq!(parsed.domain(), Some("git.sr.ht"));
    }

    #[test]
    fn forge_identity_none_for_path_indirect_resource() {
        // Resource(Git) URLs do not have a guaranteed owner/repo/domain
        // triple; those are extracted ad hoc from the URL string and not
        // part of the kind's structure, so forge_identity returns None.
        for uri in [
            "path:./foo",
            "flake:nixpkgs",
            "git+https://example.com/owner/repo",
        ] {
            let parsed: FlakeRef = uri.parse().unwrap();
            assert!(parsed.forge_identity().is_none(), "expected None for {uri}",);
        }
    }

    #[rstest]
    #[case(
        "github:nixos/nixpkgs",
        Some("nixos"),
        Some("nixpkgs"),
        Some("github.com")
    )]
    #[case("gitlab:owner/repo", Some("owner"), Some("repo"), Some("gitlab.com"))]
    #[case(
        "sourcehut:user/project",
        Some("user"),
        Some("project"),
        Some("git.sr.ht")
    )]
    #[case(
        "git+https://example.com/a/b",
        Some("a"),
        Some("b"),
        Some("example.com")
    )]
    #[case("path:./foo", None, None, None)]
    #[case("flake:nixpkgs", None, None, None)]
    fn identity_accessors(
        #[case] uri: &str,
        #[case] owner: Option<&str>,
        #[case] repo: Option<&str>,
        #[case] domain: Option<&str>,
    ) {
        let parsed: FlakeRef = uri.parse().unwrap();
        assert_eq!(parsed.owner(), owner, "owner mismatch for {uri}");
        assert_eq!(parsed.repo(), repo, "repo mismatch for {uri}");
        assert_eq!(parsed.domain(), domain, "domain mismatch for {uri}");
    }

    #[rstest]
    #[case("github:nixos/nixpkgs", RefKind::None, false)]
    #[case("github:nixos/nixpkgs/release-23.05", RefKind::Ref, false)]
    #[case(
        "github:nixos/nixpkgs/abc1234567890123456789012345678901234567",
        RefKind::Rev,
        true
    )]
    #[case(
        "flake:nixpkgs/release-23.05/abc1234567890123456789012345678901234567",
        RefKind::Both,
        true
    )]
    #[case(
        "github:nixos/nixpkgs?rev=abc1234567890123456789012345678901234567",
        RefKind::Rev,
        true
    )]
    fn ref_kind_and_pinning(
        #[case] uri: &str,
        #[case] expected_kind: RefKind,
        #[case] pinned: bool,
    ) {
        let parsed: FlakeRef = uri.parse().unwrap();
        assert_eq!(
            parsed.ref_kind(),
            expected_kind,
            "ref_kind mismatch for {uri}"
        );
        assert_eq!(
            parsed.is_pinned_to_rev(),
            pinned,
            "is_pinned_to_rev mismatch for {uri}",
        );
    }

    #[test]
    fn ref_or_rev_prefers_rev_when_pinned() {
        let parsed: FlakeRef =
            "flake:nixpkgs/release-23.05/abc1234567890123456789012345678901234567"
                .parse()
                .unwrap();
        // When both are populated, the "what does this resolve to?" answer
        // is the pinned rev; that's the canonical lookup.
        assert_eq!(
            parsed.ref_or_rev(),
            Some("abc1234567890123456789012345678901234567"),
        );
        assert_eq!(parsed.ref_(), Some("release-23.05"));
        assert_eq!(
            parsed.rev(),
            Some("abc1234567890123456789012345678901234567")
        );
    }
}

#[cfg(test)]
mod builders {
    //! Consuming builders (`with_*`, `without_pin`, `into_uri`) collapse the
    //! parse -> set -> `to_string` pattern into a single expression.
    use super::*;

    #[test]
    fn with_ref_round_trip_path_component() {
        let updated = "github:nixos/nixpkgs"
            .parse::<FlakeRef>()
            .unwrap()
            .with_ref(Some("release-23.05".into()))
            .into_uri();
        assert_eq!(updated, "github:nixos/nixpkgs/release-23.05");
    }

    #[test]
    fn with_rev_promotes_path_component_to_three_segment_for_indirect() {
        // Adding a rev to an Indirect that already has a ref produces the
        // canonical Nix three-segment form.
        let updated = "flake:nixpkgs/release-23.05"
            .parse::<FlakeRef>()
            .unwrap()
            .with_rev(Some("abc1234567890123456789012345678901234567".into()))
            .into_uri();
        assert_eq!(
            updated,
            "flake:nixpkgs/release-23.05/abc1234567890123456789012345678901234567",
        );
    }

    #[test]
    fn without_pin_clears_rev_keeps_ref() {
        let updated = "flake:nixpkgs/release-23.05/abc1234567890123456789012345678901234567"
            .parse::<FlakeRef>()
            .unwrap()
            .without_pin()
            .into_uri();
        assert_eq!(updated, "flake:nixpkgs/release-23.05");
    }

    #[test]
    fn with_rev_on_resource_flips_to_query_parameter() {
        let updated = "git+https://github.com/owner/repo"
            .parse::<FlakeRef>()
            .unwrap()
            .with_rev(Some("abc1234567890123456789012345678901234567".into()))
            .into_uri();
        assert_eq!(
            updated,
            "git+https://github.com/owner/repo?rev=abc1234567890123456789012345678901234567",
        );
    }

    #[test]
    fn with_fragment_round_trip() {
        let updated = "github:nixos/nixpkgs"
            .parse::<FlakeRef>()
            .unwrap()
            .with_fragment(Some("hello".into()))
            .into_uri();
        assert_eq!(updated, "github:nixos/nixpkgs#hello");
    }

    #[test]
    fn with_ref_then_with_rev_chains_on_gitforge() {
        // Both setters compose: each writes to its own typed slot, neither
        // clobbers the other's. `Display` for `GitForge` in `PathComponent`
        // form renders ref preferentially, so this asserts on the typed
        // slots (the source of truth) rather than on the rendered URI.
        let updated = "github:nixos/nixpkgs"
            .parse::<FlakeRef>()
            .unwrap()
            .with_ref(Some("release-23.05".into()))
            .with_rev(Some("b2df4e4e80e04cbb33a350f87717f4bd6140d298".into()));

        assert_eq!(updated.ref_(), Some("release-23.05"));
        assert_eq!(
            updated.rev(),
            Some("b2df4e4e80e04cbb33a350f87717f4bd6140d298")
        );
    }

    #[test]
    fn with_ref_then_with_rev_chains_on_indirect() {
        // Indirect renders both via the three-segment `flake:id/ref/rev`
        // form.
        let updated = "flake:nixpkgs"
            .parse::<FlakeRef>()
            .unwrap()
            .with_ref(Some("release-23.05".into()))
            .with_rev(Some("b2df4e4e80e04cbb33a350f87717f4bd6140d298".into()))
            .into_uri();

        assert_eq!(
            updated,
            "flake:nixpkgs/release-23.05/b2df4e4e80e04cbb33a350f87717f4bd6140d298",
        );
    }
}

#[cfg(test)]
mod https_github_classification {
    //! Plain `https://github.com/...` URLs do NOT auto-promote to
    //! `GitForge(GitHub)`. They classify through the same
    //! tarball-extension auto-classifier that any other `https://` URL
    //! does. Matches Nix's behaviour: the github scheme is gated on
    //! `url.scheme == "github"`, so a bare HTTPS URL only ever reaches
    //! the curl-based opaque-tarball/file path.
    //!
    //! Regression: a `flake.lock` whose `original` is
    //! `{ type = "tarball"; url = "https://github.com/NixOS/nixpkgs/pull/483360.diff"; }`
    //! must NOT round-trip through nix-uri as a github forge: Nix
    //! cannot fetch back a `github:NixOS/nixpkgs/pull/483360.diff`
    //! shape.
    use super::*;

    #[test]
    fn https_github_with_pull_path_does_not_reclassify_to_github_forge() {
        let url = "https://github.com/NixOS/nixpkgs/pull/483360.diff";
        let parsed: FlakeRef = url.parse().unwrap();
        assert!(
            !matches!(parsed.kind(), FlakeRefType::GitForge(_)),
            "expected non-GitForge classification for {url}, got {:?}",
            *parsed.kind(),
        );
        assert_eq!(parsed.to_string(), url);
    }

    #[test]
    fn https_github_owner_repo_is_resource_not_gitforge() {
        let url = "https://github.com/nixos/nixpkgs";
        let parsed: FlakeRef = url.parse().unwrap();
        assert!(
            !matches!(parsed.kind(), FlakeRefType::GitForge(_)),
            "bare https://github.com/<o>/<r> must not auto-promote to GitForge, got {:?}",
            *parsed.kind(),
        );
        assert_eq!(parsed.to_string(), url);
    }

    #[test]
    fn https_github_archive_tarball_remains_resource() {
        let url = "https://github.com/user/repo/archive/main.tar.gz";
        let parsed: FlakeRef = url.parse().unwrap();
        assert!(matches!(parsed.kind(), FlakeRefType::Resource(_)));
        assert_eq!(parsed.to_string(), url);
    }
}

#[cfg(test)]
mod ref_rev_validation {
    //! Public-surface coverage for two `GitForge` / query-rev validation
    //! contracts:
    //!
    //! - `GitForge` (`github` / `gitlab` / `sourcehut`) inputs that combine
    //!   `ref` and `rev` in any path-component or query-string combination
    //!   are rejected with `FieldConflict { left: "ref", right: "rev" }`,
    //!   matching Nix's rejection of the same shape on git-archive URLs.
    //!   Indirect's three-segment form and `Resource(Git)`'s
    //!   `?ref=&rev=` shape stay legitimate.
    //! - A `?rev=` value that is not exactly 40 ASCII hex digits is
    //!   rejected with `InvalidValue { field: "rev", .. }`. The
    //!   path-component side already validated via `looks_like_rev`; the
    //!   query side was the unguarded ingestion path.
    use super::*;
    use crate::error::NixUriError;
    use cool_asserts::assert_matches;
    use rstest::rstest;

    const HEX40: &str = "b2df4e4e80e04cbb33a350f87717f4bd6140d298";

    #[rstest]
    #[case::github_both_in_query(
        "github:owner/repo?ref=main&rev=b2df4e4e80e04cbb33a350f87717f4bd6140d298"
    )]
    #[case::github_ref_path_rev_query(
        "github:owner/repo/main?rev=b2df4e4e80e04cbb33a350f87717f4bd6140d298"
    )]
    #[case::github_rev_path_ref_query(
        "github:owner/repo/b2df4e4e80e04cbb33a350f87717f4bd6140d298?ref=main"
    )]
    #[case::gitlab_both_in_query(
        "gitlab:owner/repo?ref=main&rev=b2df4e4e80e04cbb33a350f87717f4bd6140d298"
    )]
    #[case::gitlab_ref_path_rev_query(
        "gitlab:owner/repo/main?rev=b2df4e4e80e04cbb33a350f87717f4bd6140d298"
    )]
    #[case::gitlab_rev_path_ref_query(
        "gitlab:owner/repo/b2df4e4e80e04cbb33a350f87717f4bd6140d298?ref=main"
    )]
    #[case::sourcehut_both_in_query(
        "sourcehut:~owner/repo?ref=main&rev=b2df4e4e80e04cbb33a350f87717f4bd6140d298"
    )]
    #[case::sourcehut_ref_path_rev_query(
        "sourcehut:~owner/repo/main?rev=b2df4e4e80e04cbb33a350f87717f4bd6140d298"
    )]
    #[case::sourcehut_rev_path_ref_query(
        "sourcehut:~owner/repo/b2df4e4e80e04cbb33a350f87717f4bd6140d298?ref=main"
    )]
    fn gitforge_rejects_ref_and_rev_together(#[case] uri: &str) {
        // Matches Nix's rejection of git-archive URLs that combine ref
        // and rev. FieldConflict surfaces the structural relationship
        // (two mutually exclusive fields were both populated), distinct
        // from a value-shape failure.
        assert_matches!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::FieldConflict {
                left: "ref",
                right: "rev",
            }),
            "expected mutual-exclusion rejection for {uri}",
        );
    }

    #[rstest]
    #[case::github("github:owner/repo?rev=not-a-hash")]
    #[case::git_https("git+https://example.com/owner/repo?rev=not-a-hash")]
    #[case::hg_https("hg+https://example.com/repo?rev=zzzz")]
    #[case::indirect("flake:nixpkgs?rev=main")]
    #[case::gitlab("gitlab:owner/repo?rev=not-a-hash")]
    #[case::sourcehut("sourcehut:~owner/repo?rev=not-a-hash")]
    #[case::short_hex("github:owner/repo?rev=abc123")]
    #[case::between_40_and_64_hex(
        "github:owner/repo?rev=b2df4e4e80e04cbb33a350f87717f4bd6140d29800000"
    )]
    #[case::sixty_five_hex(
        "github:owner/repo?rev=00000000000000000000000000000000000000000000000000000000000000000"
    )]
    #[case::sixty_three_hex(
        "github:owner/repo?rev=000000000000000000000000000000000000000000000000000000000000000"
    )]
    fn query_rev_must_be_40_or_64_hex(#[case] uri: &str) {
        // Matches Nix's accepted commit-hash shapes: SHA-1 (40 hex) or
        // SHA-256 (64 hex). Anything else surfaces as InvalidValue with
        // the algorithm-aware diagnostic.
        assert_matches!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::InvalidValue {
                field: "rev",
                reason,
            }) if reason == "expected 40-hex (SHA-1) or 64-hex (SHA-256) commit",
            "expected hex-validation rejection for {uri}",
        );
    }

    // GitLab and SourceHut `?ref=...` (the ref-only query shape) have no
    // existing positive coverage; the github/git+ siblings are already
    // pinned by `set_ref_preserves_query_parameter_location`,
    // `set_rev_preserves_location`, and the `canonical_round_trip`
    // rstest. Pin the two missing forges here so the new check is shown
    // not to over-reject them.
    #[rstest]
    #[case::gitlab_query_ref("gitlab:owner/repo?ref=main")]
    #[case::sourcehut_query_ref("sourcehut:~owner/repo?ref=main")]
    fn ref_only_query_still_parses_for_each_forge(#[case] uri: &str) {
        let parsed = uri
            .parse::<FlakeRef>()
            .expect("input must continue to parse cleanly");
        assert_eq!(parsed.to_string(), uri, "round-trip mismatch for {uri}");
    }

    #[test]
    fn indirect_path_component_three_segment_still_parses() {
        // Nix's `flake:id/ref/rev` form populates BOTH ref and rev
        // legitimately; the GitForge exclusion does not extend to
        // Indirect. Pin that.
        let uri = format!("flake:nixpkgs/release-23.05/{HEX40}");
        let parsed: FlakeRef = uri.parse().unwrap();
        assert_eq!(parsed.ref_(), Some("release-23.05"));
        assert_eq!(parsed.rev(), Some(HEX40));
    }

    #[test]
    fn resource_git_with_ref_and_rev_still_parses() {
        // Resource(Git) explicitly supports `?ref=branch&rev=hex` (you
        // can pin a rev and remember which branch it came from). This
        // is NOT the GitForge case; do not reject.
        let uri = "git+https://git.somehost.tld/user/path?ref=branch&rev=fdc8ef970de2b4634e1b3dca296e1ed918459a9e";
        let parsed: FlakeRef = uri.parse().unwrap();
        assert_eq!(parsed.to_string(), uri);
    }
}

#[cfg(test)]
mod path_authority_and_rev {
    //! Public-surface coverage for the `Path` arm:
    //!
    //! - `path://...` is a malformed shape Nix rejects with a
    //!   "path URL should not have an authority" diagnostic; nix-uri
    //!   refuses any `path:` input where the body begins with `//`.
    //! - `?rev=<40hex>` on a `path:` input routes into the typed `rev`
    //!   slot and Display re-emits it; locked store-path inputs of the
    //!   form `path:/nix/store/...?rev=...` round-trip cleanly.
    use super::*;
    use crate::error::{NixUriError, UnsupportedReason};
    use cool_asserts::assert_matches;
    use rstest::rstest;

    const HEX40: &str = "b2df4e4e80e04cbb33a350f87717f4bd6140d298";

    #[rstest]
    #[case::host("path://somehost/abs/path")]
    #[case::host_no_path("path://x")]
    fn path_authority_rejected(#[case] uri: &str) {
        assert_matches!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::Unsupported(UnsupportedReason::Authority {
                scheme: "path",
            })),
            "expected authority rejection for {uri}",
        );
    }

    /// Nix rejects `path://` only when the URL authority's host is
    /// non-empty. An empty authority (the body literally begins `///`
    /// or is just `//`) is accepted and decodes to the trailing path.
    /// Pin both the parse acceptance and the Display round-trip.
    #[rstest]
    #[case::triple_slash("path:///abs/path")]
    #[case::quad_slash("path:////a")]
    fn path_triple_slash_parses_as_absolute_path(#[case] uri: &str) {
        let parsed: FlakeRef = uri
            .parse()
            .unwrap_or_else(|e| panic!("empty-authority path must parse: {uri} -> {e}"));
        assert_matches!(parsed.kind(), FlakeRefType::Path { rev: None, .. });
        assert_eq!(parsed.to_string(), uri, "round-trip mismatch for {uri}");
        let reparsed: FlakeRef = parsed.to_string().parse().unwrap();
        assert_eq!(parsed, reparsed, "parse-Display-parse not stable for {uri}");
    }

    #[test]
    fn path_with_authority_host_rejects() {
        assert_matches!(
            "path://host/abs".parse::<FlakeRef>(),
            Err(NixUriError::Unsupported(UnsupportedReason::Authority {
                scheme: "path",
            })),
        );
    }

    #[rstest]
    #[case::abs("path:/foo/bar")]
    #[case::abs_trailing("path:/home/kenji/.config/dotfiles/")]
    #[case::cwd("path:./relative")]
    #[case::parent("path:..")]
    #[case::single_dot("path:.")]
    fn path_non_authority_still_parses(#[case] uri: &str) {
        let parsed: FlakeRef = uri
            .parse()
            .unwrap_or_else(|e| panic!("path body must continue to parse: {uri} -> {e}"));
        assert_eq!(parsed.to_string(), uri, "round-trip mismatch for {uri}");
    }

    #[rstest]
    #[case::store_path(&format!("path:/nix/store/abc?rev={HEX40}"))]
    #[case::abs(&format!("path:/var/cache?rev={HEX40}"))]
    #[case::with_trailing_slash(&format!("path:/home/kenji/?rev={HEX40}"))]
    fn path_rev_query_round_trips(#[case] uri: &str) {
        let parsed: FlakeRef = uri.parse().expect("path with ?rev= must parse");
        assert_eq!(parsed.rev(), Some(HEX40), "rev not stored for {uri}");
        assert_eq!(parsed.to_string(), uri, "round-trip mismatch for {uri}");
        let reparsed: FlakeRef = parsed.to_string().parse().unwrap();
        assert_eq!(parsed, reparsed, "parse-Display-parse not stable for {uri}");
    }

    #[test]
    fn path_rev_with_dir_keeps_alphabetical_query() {
        // `dir` typed slot + `rev` from kind both land in the alphabetical
        // query block; dir < rev lexicographically.
        let uri = format!("path:/abs/path?dir=sub&rev={HEX40}");
        let parsed: FlakeRef = uri.parse().expect("must parse");
        assert_eq!(parsed.to_string(), uri);
    }
}

#[cfg(test)]
mod percent_encoding_round_trip {
    //! Pin the encoder/decoder pair against Nix's two encoding contracts:
    //! query strings keep `unreserved + ":@/?"` raw, while the fragment
    //! encodes the four extra bytes `:@/?` too. The tests cover both
    //! sides plus the strict-decode contract (a stray `%` not followed
    //! by two hex digits rejects rather than passing through).
    use crate::{FlakeRef, NixUriError};
    use rstest::rstest;

    #[rstest]
    #[case::space("github:o/r?dir=foo%20bar", "foo bar")]
    #[case::percent("github:o/r?dir=foo%25bar", "foo%bar")]
    #[case::semicolon("github:o/r?dir=foo%3Bbar", "foo;bar")]
    #[case::plus("github:o/r?dir=foo%2Bbar", "foo+bar")]
    #[case::ampersand("github:o/r?dir=foo%26bar", "foo&bar")]
    #[case::equals("github:o/r?dir=foo%3Dbar", "foo=bar")]
    #[case::hash("github:o/r?dir=foo%23bar", "foo#bar")]
    #[case::non_ascii("github:o/r?dir=f%C3%96%C3%B6", "fÖö")]
    fn query_value_round_trips_for_encoded_byte(#[case] input: &str, #[case] decoded: &str) {
        let parsed: FlakeRef = input.parse().expect("input must parse");
        let dir_value = parsed
            .params()
            .entries()
            .into_iter()
            .find(|(k, _)| *k == "dir")
            .map(|(_, v)| v.to_string());
        assert_eq!(dir_value, Some(decoded.to_string()));
        assert_eq!(parsed.to_string(), input);
    }

    #[rstest]
    #[case::colon_in_value("github:o/r?dir=foo:bar")]
    #[case::at_in_value("github:o/r?dir=foo@bar")]
    #[case::slash_in_value("github:o/r?dir=foo/bar")]
    fn allowed_query_chars_remain_unencoded(#[case] input: &str) {
        let parsed: FlakeRef = input.parse().expect("input must parse");
        assert_eq!(parsed.to_string(), input);
    }

    #[rstest]
    #[case::space("github:o/r#default%20package", "default package")]
    #[case::percent("github:o/r#a%25b", "a%b")]
    #[case::non_ascii("github:o/r#f%C3%96%C3%B6", "fÖö")]
    #[case::question_mark_in_fragment("github:o/r#a%3Fb", "a?b")]
    #[case::slash_in_fragment("github:o/r#a%2Fb", "a/b")]
    fn fragment_round_trips_for_encoded_byte(#[case] input: &str, #[case] decoded: &str) {
        let parsed: FlakeRef = input.parse().expect("input must parse");
        assert_eq!(parsed.fragment(), Some(decoded));
        assert_eq!(parsed.to_string(), input);
    }

    #[rstest]
    #[case::truncated_one_hex("github:o/r?dir=%2")]
    #[case::truncated_no_hex("github:o/r?dir=%")]
    #[case::non_hex("github:o/r?dir=%XY")]
    #[case::non_hex_partial("github:o/r?dir=%2Z")]
    fn malformed_query_value_percent_encoding_rejected(#[case] input: &str) {
        match input.parse::<FlakeRef>() {
            Err(NixUriError::InvalidUrl(_)) => {}
            other => panic!("expected InvalidUrl for {input}, got {other:?}"),
        }
    }

    #[rstest]
    #[case::truncated("github:o/r#a%2")]
    #[case::non_hex("github:o/r#a%XY")]
    fn malformed_fragment_percent_encoding_rejected(#[case] input: &str) {
        match input.parse::<FlakeRef>() {
            Err(NixUriError::InvalidUrl(_)) => {}
            other => panic!("expected InvalidUrl for {input}, got {other:?}"),
        }
    }

    #[test]
    fn arbitrary_param_value_round_trips_with_space() {
        // `name` is a real Git allowedAttr (no typed slot in
        // `LocationParameters`), so it lands in the arbitrary bag where
        // the encoder pair is exercised end-to-end.
        let input = "git+https://example.com/repo?name=hello%20world";
        let parsed: FlakeRef = input.parse().unwrap();
        assert_eq!(parsed.to_string(), input);
    }
}

#[cfg(test)]
mod ref_rev_builders {
    //! Tests for the `pin_to_rev` and `try_with_ref` / `try_with_rev`
    //! ergonomic builders. The `pin_to_rev` shape exists because
    //! `with_rev(Some(_))` on a `RefLocation::PathComponent` `GitForge`
    //! that already carries a `ref_` silently drops the rev (see the
    //! `ref_.or(rev)` Display arm in `fr_type.rs`). The `try_*` shape
    //! is the loud opt-in alternative to the silent no-op `with_*`
    //! builders for callers who want the round-trip mismatch surfaced
    //! at the call site.
    use super::*;
    use crate::{NixUriError, UnsupportedReason};

    const HEX40: &str = "b2df4e4e80e04cbb33a350f87717f4bd6140d298";

    #[test]
    fn pin_to_rev_clears_path_component_ref() {
        // Regression: with_rev(Some(rev)) on a parsed `github:foo/bar/main`
        // (which carries ref_=Some("main")) leaves both ref_ and rev set;
        // the GitForge Display arm at fr_type.rs renders `ref_.or(rev)` for
        // RefLocation::PathComponent, so the rev silently disappears from
        // the round-trip string. `pin_to_rev` clears the ref slot first so
        // the rev wins.
        let parsed: FlakeRef = "github:foo/bar/main".parse().unwrap();
        let pinned = parsed.pin_to_rev(HEX40.to_string());
        assert_eq!(pinned.ref_(), None);
        assert_eq!(pinned.rev(), Some(HEX40));
        let rendered = pinned.to_string();
        assert!(rendered.contains(HEX40), "expected rev in {rendered}");
        assert!(!rendered.contains("main"), "ref leaked into {rendered}");
    }

    #[test]
    fn pin_to_rev_replaces_query_param_ref() {
        let parsed: FlakeRef = "github:foo/bar?ref=main".parse().unwrap();
        let pinned = parsed.pin_to_rev(HEX40.to_string());
        assert_eq!(pinned.ref_(), None);
        assert_eq!(pinned.rev(), Some(HEX40));
        let rendered = pinned.to_string();
        assert!(rendered.contains(HEX40), "expected rev in {rendered}");
        assert!(!rendered.contains("ref="), "ref= leaked into {rendered}");
    }

    #[test]
    fn pin_to_rev_sets_rev_on_path() {
        // `Path` has a typed `rev` slot (`Path { path, rev }`), so pinning
        // is meaningful: it writes the rev. There is no ref slot to clear.
        let parsed: FlakeRef = "path:/x/y".parse().unwrap();
        let pinned = parsed.pin_to_rev(HEX40.to_string());
        assert_eq!(pinned.rev(), Some(HEX40));
    }

    #[test]
    fn try_with_ref_path_returns_unsupported() {
        // Nix's path scheme does not accept `ref`; setting one would
        // render a string the parser would reject (round-trip break).
        // The loud opt-in surfaces this as a typed error.
        let parsed: FlakeRef = "path:/x/y".parse().unwrap();
        let result = parsed.try_with_ref(Some("main".into()));
        match result {
            Err(NixUriError::Unsupported(UnsupportedReason::Field { field, .. })) => {
                assert_eq!(field, "ref");
            }
            other => panic!("expected Unsupported(Field {{ field: \"ref\" }}), got {other:?}"),
        }
    }

    #[test]
    fn try_with_ref_tarball_returns_unsupported() {
        // Nix's tarball/file schemes do not accept `ref`.
        let parsed: FlakeRef = "tarball+https://example.com/foo.tar.gz".parse().unwrap();
        let result = parsed.try_with_ref(Some("v1".into()));
        match result {
            Err(NixUriError::Unsupported(UnsupportedReason::Field { field, .. })) => {
                assert_eq!(field, "ref");
            }
            other => panic!("expected Unsupported(Field {{ field: \"ref\" }}), got {other:?}"),
        }
    }

    #[test]
    fn try_with_ref_github_succeeds() {
        let parsed: FlakeRef = "github:foo/bar".parse().unwrap();
        let updated = parsed
            .try_with_ref(Some("main".into()))
            .expect("github accepts ref");
        assert_eq!(updated.ref_(), Some("main"));
    }

    #[test]
    fn try_with_ref_clear_is_always_ok() {
        // Clearing has no Nix-level implications, so try_with_ref(None)
        // succeeds even on kinds without a ref slot.
        let parsed: FlakeRef = "path:/x/y".parse().unwrap();
        let cleared = parsed.try_with_ref(None).expect("clear is a no-op");
        assert_eq!(cleared.ref_(), None);
    }

    #[test]
    fn try_with_rev_path_succeeds() {
        // Nix's path scheme accepts `rev`, so try_with_rev surfaces
        // no error.
        let parsed: FlakeRef = "path:/x/y".parse().unwrap();
        let pinned = parsed
            .try_with_rev(Some(HEX40.into()))
            .expect("path accepts rev");
        assert_eq!(pinned.rev(), Some(HEX40));
    }

    #[test]
    fn with_ref_silent_noop_path() {
        // Regression guard for the existing infallible builder: setting
        // a ref on Path is silently dropped today (set_ref's Path arm is
        // a no-op). The try_* variant is the loud opt-in alternative.
        let parsed: FlakeRef = "path:/x/y".parse().unwrap();
        let updated = parsed.with_ref(Some("main".into()));
        assert_eq!(updated.ref_(), None, "with_ref must remain a no-op on Path");
    }
}

#[cfg(test)]
mod canonical_string {
    //! Tests for [`FlakeRef::to_canonical_string`].
    //!
    //! Pins the per-scheme canonicalisation rules and guards `Display`
    //! from drifting along with them.

    use super::*;

    const HEX40: &str = "0000000000000000000000000000000000000000";

    /// Round-trip helper: every canonical string this library produces
    /// must re-parse as a `FlakeRef`. (It need not equal the input
    /// `FlakeRef` byte-for-byte; canonical-form may drop fields like
    /// `?ref=` location or `?allRefs=1`.)
    fn assert_canonical_reparses(input: &str) {
        let parsed: FlakeRef = input.parse().expect("input parses");
        let canonical = parsed.to_canonical_string();
        let _: FlakeRef = canonical
            .parse()
            .unwrap_or_else(|e| panic!("canonical {canonical:?} failed to re-parse: {e}"));
    }

    // GitForge `?ref=` / `?rev=` canonicalises to path-component form.

    #[test]
    fn github_ref_query_canonicalises_to_path_component() {
        let input = "github:nixos/nixpkgs?ref=nixos-23.11";
        let parsed: FlakeRef = input.parse().unwrap();
        assert_eq!(
            parsed.to_canonical_string(),
            "github:nixos/nixpkgs/nixos-23.11"
        );
        // Display-unchanged regression guard.
        assert_eq!(parsed.to_string(), input);
        assert_canonical_reparses(input);
    }

    #[test]
    fn github_rev_query_canonicalises_to_path_component() {
        let input = "github:nixos/nixpkgs?rev=0000000000000000000000000000000000000000";
        let parsed: FlakeRef = input.parse().unwrap();
        assert_eq!(
            parsed.to_canonical_string(),
            "github:nixos/nixpkgs/0000000000000000000000000000000000000000"
        );
        assert_eq!(parsed.to_string(), input);
        assert_canonical_reparses(input);
    }

    #[test]
    fn github_path_component_ref_unchanged() {
        // Already-canonical input survives canonicalisation as-is.
        let input = "github:nixos/nixpkgs/main";
        let parsed: FlakeRef = input.parse().unwrap();
        assert_eq!(parsed.to_canonical_string(), input);
        assert_eq!(parsed.to_string(), input);
    }

    #[test]
    fn gitlab_ref_query_canonicalises() {
        let input = "gitlab:foo/bar?ref=v1.0";
        let parsed: FlakeRef = input.parse().unwrap();
        assert_eq!(parsed.to_canonical_string(), "gitlab:foo/bar/v1.0");
        assert_eq!(parsed.to_string(), input);
    }

    #[test]
    fn sourcehut_ref_query_canonicalises() {
        let input = "sourcehut:~user/repo?ref=branch";
        let parsed: FlakeRef = input.parse().unwrap();
        assert_eq!(parsed.to_canonical_string(), "sourcehut:~user/repo/branch");
        assert_eq!(parsed.to_string(), input);
    }

    #[test]
    fn github_canonical_keeps_host_and_nar_hash() {
        // `host` and `narHash` are the two query keys Nix emits on a
        // canonical git-archive URL. Everything else (dir,
        // lastModified, revCount, arbitrary) is dropped.
        let input = "github:nixos/nixpkgs/main?host=ghe.example.com&narHash=sha256-abc";
        let parsed: FlakeRef = input.parse().unwrap();
        assert_eq!(
            parsed.to_canonical_string(),
            "github:nixos/nixpkgs/main?host=ghe.example.com&narHash=sha256-abc"
        );
    }

    #[test]
    fn github_canonical_picks_rev_over_ref() {
        // Construct a pathological state where both ref and rev are
        // populated. Nix asserts they cannot coexist on a git-archive
        // URL, so this is nonsensical, but we need a deterministic
        // answer if a caller ever wires both via the builder; pick rev
        // (rev is the more-specific pin).
        let mut forge = GitForge {
            platform: GitForgePlatform::GitHub,
            owner: "nixos".into(),
            repo: "nixpkgs".into(),
            ref_: Some("main".into()),
            rev: Some(HEX40.into()),
            location: RefLocation::PathComponent,
        };
        // The proptest generator deliberately avoids this shape; we
        // synthesise it directly.
        forge.location = RefLocation::PathComponent;
        let f = FlakeRef::default().with_kind(FlakeRefType::GitForge(forge));
        assert_eq!(
            f.to_canonical_string(),
            format!("github:nixos/nixpkgs/{HEX40}")
        );
    }

    // Typed Git booleans only emit on the truthy branch, and
    // `allRefs` is never emitted.

    #[test]
    fn git_all_refs_dropped_on_canonical() {
        let input = "git+https://github.com/nixos/nixpkgs?allRefs=1";
        let parsed: FlakeRef = input.parse().unwrap();
        // Nix's canonical git URL does not include allRefs in the
        // serialised query.
        assert_eq!(
            parsed.to_canonical_string(),
            "git+https://github.com/nixos/nixpkgs"
        );
        // Display still preserves the round-trip.
        assert_eq!(parsed.to_string(), input);
        assert_canonical_reparses(input);
    }

    #[test]
    fn git_lfs_truthy_kept() {
        let input = "git+https://example.com/repo?lfs=1";
        let parsed: FlakeRef = input.parse().unwrap();
        assert_eq!(parsed.to_canonical_string(), input);
        assert_eq!(parsed.to_string(), input);
    }

    #[test]
    fn git_lfs_falsy_dropped() {
        let input = "git+https://example.com/repo?lfs=0";
        let parsed: FlakeRef = input.parse().unwrap();
        assert_eq!(parsed.to_canonical_string(), "git+https://example.com/repo");
        assert_eq!(parsed.to_string(), input);
    }

    #[test]
    fn git_submodules_truthy_kept() {
        let input = "git+https://example.com/repo?submodules=1";
        let parsed: FlakeRef = input.parse().unwrap();
        assert_eq!(parsed.to_canonical_string(), input);
        assert_eq!(parsed.to_string(), input);
    }

    #[test]
    fn git_shallow_truthy_kept() {
        let input = "git+https://example.com/repo?shallow=1";
        let parsed: FlakeRef = input.parse().unwrap();
        assert_eq!(parsed.to_canonical_string(), input);
        assert_eq!(parsed.to_string(), input);
    }

    #[test]
    fn git_export_ignore_truthy_kept() {
        let input = "git+https://example.com/repo?exportIgnore=1";
        let parsed: FlakeRef = input.parse().unwrap();
        assert_eq!(parsed.to_canonical_string(), input);
    }

    #[test]
    fn git_verify_commit_truthy_kept() {
        let input = "git+https://example.com/repo?verifyCommit=1";
        let parsed: FlakeRef = input.parse().unwrap();
        assert_eq!(parsed.to_canonical_string(), input);
    }

    #[test]
    fn git_locked_attrs_dropped() {
        // narHash, lastModified, revCount are not part of Nix's
        // canonical git URL output; canonical drops them.
        let input = "git+https://example.com/repo?lastModified=42&narHash=sha256-x&revCount=7";
        let parsed: FlakeRef = input.parse().unwrap();
        assert_eq!(parsed.to_canonical_string(), "git+https://example.com/repo");
    }

    #[test]
    fn git_ref_and_rev_canonical_alphabetised() {
        // Nix emits both ref and rev on the same canonical git URL
        // when set; canonical sort keeps ref before rev.
        let input = format!("git+https://example.com/repo?ref=main&rev={HEX40}");
        let parsed: FlakeRef = input.parse().unwrap();
        assert_eq!(parsed.to_canonical_string(), input);
    }

    // Resource(Mercurial): only ref/rev survive canonicalisation.

    #[test]
    fn hg_canonical_keeps_only_ref_and_rev() {
        let input = "hg+https://example.com/repo?ref=main";
        let parsed: FlakeRef = input.parse().unwrap();
        assert_eq!(parsed.to_canonical_string(), input);
    }

    // Indirect / Path / File / Tarball: canonical delegates to Display.

    #[test]
    fn indirect_canonical_matches_display() {
        for input in ["flake:nixpkgs", "flake:nixpkgs/main", "flake:nixos/nixpkgs"] {
            let parsed: FlakeRef = input.parse().unwrap();
            assert_eq!(parsed.to_canonical_string(), parsed.to_string(), "{input}");
            assert_eq!(parsed.to_canonical_string(), input);
        }
    }

    #[test]
    fn path_canonical_matches_display() {
        for input in ["path:./foo", "path:/abs/path"] {
            let parsed: FlakeRef = input.parse().unwrap();
            assert_eq!(parsed.to_canonical_string(), parsed.to_string(), "{input}");
            assert_eq!(parsed.to_canonical_string(), input);
        }
    }

    #[test]
    fn fragment_survives_canonicalisation() {
        let input = "github:nixos/nixpkgs/main#hello";
        let parsed: FlakeRef = input.parse().unwrap();
        assert_eq!(
            parsed.to_canonical_string(),
            "github:nixos/nixpkgs/main#hello"
        );
        assert_eq!(parsed.to_string(), input);
    }
}

#[cfg(test)]
mod historical_seed_round_trip {
    //! Round-trip pins for shapes once captured in
    //! `proptest-regressions/flakeref/proptest.txt`. Each test builds
    //! the value and asserts the round-trip property
    //! `flake_ref.to_string().parse() == Ok(flake_ref)`.
    use super::*;

    fn assert_round_trip(value: &FlakeRef) {
        let displayed = value.to_string();
        let parsed: FlakeRef = displayed
            .parse()
            .unwrap_or_else(|e| panic!("Display output {displayed:?} failed to parse: {e}"));
        assert_eq!(*value, parsed, "round-trip mismatch for {displayed:?}");
    }

    /// `path://` with an empty authority parses to a `Path` whose body
    /// is the literal `//`. Nix's URL parser admits the empty-authority
    /// form (it only rejects an authority with a non-empty host). The
    /// internal byte-for-byte round-trip must hold so a value
    /// constructed this way comes back identical.
    #[test]
    fn path_double_slash_round_trips() {
        let value = FlakeRef::new(FlakeRefType::Path {
            path: "//".to_string(),
            rev: None,
        });
        assert_eq!(value.to_string(), "path://");
        assert_round_trip(&value);
    }
}
