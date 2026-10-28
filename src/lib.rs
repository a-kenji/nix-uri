//!
//! [nix-uri](https://crates.io/crates/nix-uri) is a rust crate that parses
//! the [nix-uri-scheme](https://nixos.org/manual/nix/stable/command-ref/new-cli/nix3-flake#url-like-syntax)
//! into a [`flakeref::FlakeRef`] struct.
//!
//! Also allows for building a `nix-uri` through the [`flakeref::FlakeRef`]
//! struct.
//!
//! Convenience functionality for working with nix `flake.nix` references (flakerefs).
//! Provides types for the generic attribute set representation, but does not parse it:
//!
//! ``` markdown
//!    {
//!      type = "github";
//!      owner = "NixOS";
//!      repo = "nixpkgs";
//!    }
//! ```
//!
//! ## Installation
//!
//! To use `nix-uri`, add it as a dependency in your `Cargo.toml` file:
//!
//! ```markdown
//! [dependencies]
//! nix-uri = "0.2.0"
//! ```
//!
//! or use `cargo add`:
//!
//! ```markdown
//! cargo add nix-uri
//! ```
//!
//! # Examples
//! Check out the examples directory, for more information, or run an example:
//!
//! ```markdown
//! cargo run --example simple
//! cargo run --example cli github:nixos/nixpkgs
//! ```
//!
//! The uri syntax representation is parsed by this library:
//! ## Example: Parsing from `github:nixos/nixpkgs`:
//!
//!  ```
//!   # use nix_uri::{FlakeRef, FlakeRefType, GitForgePlatform};
//!   let uri = "github:nixos/nixpkgs";
//!   let parsed: FlakeRef = uri.parse().unwrap();
//!   match parsed.kind() {
//!       FlakeRefType::GitForge(forge) => {
//!           assert_eq!(forge.platform, GitForgePlatform::GitHub);
//!           assert_eq!(forge.owner, "nixos");
//!           assert_eq!(forge.repo, "nixpkgs");
//!           assert!(forge.ref_.is_none() && forge.rev.is_none());
//!       }
//!       _ => panic!("expected GitForge"),
//!   }
//!   ```
//!
//!   ## Parse context
//!
//!   Which flake references are accepted depends on where they appear,
//!   see [`FlakeRef::parse`] and [`Context`]. `FromStr` and `TryFrom`
//!   parse as a flake input ([`Context::FlakeInput`]).
//!
//!   ```
//!   # use nix_uri::{Context, FlakeRef, FlakeRefType};
//!   // `inputs.<name>.url` keeps relative paths.
//!   let input = FlakeRef::parse("./vendor/foo", Context::FlakeInput).unwrap();
//!   assert!(matches!(input.kind(), FlakeRefType::Path { .. }));
//!
//!   // On its own, a flake reference takes only absolute paths and no fragment.
//!   assert!(FlakeRef::parse("./vendor/foo", Context::Standalone).is_err());
//!
//!   // A command-line installable may name an attribute after `#`.
//!   let installable = FlakeRef::parse("nixpkgs#hello", Context::Installable).unwrap();
//!   assert_eq!(installable.fragment(), Some("hello"));
//!   ```
//!
//!   The `Display` round-trip preserves the original form:
//!   ## Example: Round-tripping `github:nixos/nixpkgs`:
//!   ```
//!   # use nix_uri::FlakeRef;
//!   let uri = "github:nixos/nixpkgs";
//!   let parsed: FlakeRef = uri.parse().unwrap();
//!   assert_eq!(uri, parsed.to_string());
//!   ```

mod context;
mod error;
mod flakeref;
pub(crate) mod parser;

pub use context::Context;
pub use error::{NixUriError, NixUriResult, ParseExpected, UnsupportedReason};
pub use flakeref::{
    FlakeRef, FlakeRefType, ForgeIdentity, GitForge, GitForgePlatform, LocationParameters, RefKind,
    RefLocation, ResourceType, ResourceUrl, TransportLayer,
};
