/// Where a flake reference string appears.
///
/// The same string is accepted or rejected depending on where it comes
/// from: whether a `#fragment` is allowed, and whether a relative path is
/// kept or rejected. Every variant picks one of those call sites for
/// [`crate::FlakeRef::parse`].
///
/// Inputs without a scheme that are neither a flake id (`nixpkgs`,
/// `nixpkgs/nixos-25.05`) nor a URL fall back to a path.
/// So in [`Context::FlakeInput`] and [`Context::Installable`], a string
/// such as `nixpkgs?dir=lib` parses as the relative path `nixpkgs`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Context {
    /// A flake reference on its own, with no base directory to resolve a
    /// relative path against and no attribute path: only absolute paths,
    /// no `#fragment`. This is what `builtins.parseFlakeRef` and
    /// `builtins.getFlake` accept.
    Standalone,
    /// `inputs.<name>.url` in a `flake.nix`: relative paths are kept
    /// (`./x` parses as a path), no `#fragment`.
    #[default]
    FlakeInput,
    /// A command-line installable: a `#attrpath` fragment is allowed.
    /// Relative paths are returned unresolved. Searching upwards from them
    /// for the enclosing repository needs the filesystem and is left to
    /// the caller.
    Installable,
}

impl Context {
    pub(crate) fn allows_fragment(self) -> bool {
        matches!(self, Self::Installable)
    }

    pub(crate) fn allows_relative_path(self) -> bool {
        !matches!(self, Self::Standalone)
    }
}
