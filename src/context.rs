/// Where a flake reference string appears.
///
/// The same string is accepted or rejected depending on where it comes from. Every
/// variant picks one of those call sites for [`crate::FlakeRef::parse`]. A `#fragment` is
/// never part of a flake reference, see [`crate::FlakeRef::parse_with_fragment`] for a
/// reference followed by an attribute path.
///
/// Inputs without a scheme that are neither a flake id (`nixpkgs`, `nixpkgs/nixos-25.05`)
/// nor a URL fall back to a path. So in [`Context::FlakeInput`], a string such as
/// `nixpkgs?dir=lib` parses as the relative path `nixpkgs`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Context {
    /// A flake reference on its own, with no base directory to resolve a relative path
    /// against: only absolute paths. This is what `builtins.parseFlakeRef` and
    /// `builtins.getFlake` accept, and what [`FromStr`](std::str::FromStr) and
    /// `TryFrom<&str>` parse.
    #[default]
    Standalone,
    /// `inputs.<name>.url` in a `flake.nix`: relative paths are kept (`./x` parses as a
    /// path). They are returned unresolved: resolving them needs the directory of the
    /// `flake.nix`, which is left to the caller.
    FlakeInput,
}

impl Context {
    pub(crate) fn allows_relative_path(self) -> bool {
        matches!(self, Self::FlakeInput)
    }
}
