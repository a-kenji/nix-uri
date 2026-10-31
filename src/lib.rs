//!
//! [nix-uri](https://crates.io/crates/nix-uri) parses [flake references](https://nixos.org/manual/nix/stable/command-ref/new-cli/nix3-flake#url-like-syntax) into a [`FlakeRef`].
//!
//! A [`FlakeRef`] carries Nix's attributes: [`FlakeRef::to_attrs`] and
//! [`FlakeRef::from_attrs`] reads a node of `flake.lock` or a registry entry.
//! Its `Display` tries to mirror how Nix would render these attributes, which means
//! they are not necessarily round-trip stable.
//!
//! Example:
//!
//! ```
//! # use nix_uri::FlakeRef;
//! let flake_ref: FlakeRef = "github:NixOS/nixpkgs?dir=lib&ref=nixos-25.05".parse().unwrap();
//! assert_eq!(flake_ref.to_string(), "github:NixOS/nixpkgs/nixos-25.05?dir=lib");
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
//! ## Features
//!
//! - `serde`: `Serialize` and `Deserialize` for [`FlakeRef`] and [`Attrs`]
//!
//! # Examples
//! Check out the examples directory, for more information, or run an example:
//!
//! ```markdown
//! cargo run --example simple
//! cargo run --example cli --features serde github:nixos/nixpkgs
//! ```
//!
//! ## Example: Parsing `github:nixos/nixpkgs`
//!
//! ```
//! # use nix_uri::{FlakeRef, FlakeRefType, GitForgePlatform};
//! let parsed: FlakeRef = "github:nixos/nixpkgs".parse().unwrap();
//! match parsed.kind() {
//!     FlakeRefType::GitForge(forge) => {
//!         assert_eq!(forge.platform, GitForgePlatform::GitHub);
//!         assert_eq!(forge.owner, "nixos");
//!         assert_eq!(forge.repo, "nixpkgs");
//!         assert!(forge.ref_.is_none() && forge.rev.is_none());
//!     }
//!     _ => panic!("expected GitForge"),
//! }
//! ```
//!
//! ## Parse context
//!
//! Which flake references are accepted and how they are parsed depends on
//! the context they are in.
//! [`FlakeRef::parse`] and [`Context`].
//! `FromStr` and `TryFrom` parse as `builtins.parseFlakeRef` would do
//! ([`Context::Standalone`]). If you want to parse flake references from the CLI,
//! they should be parsed through: [`FlakeRef::parse_with_fragment`].
//!
//! ```
//! # use nix_uri::{Context, FlakeRef, FlakeRefType};
//! // `inputs.<name>.url` keeps relative paths.
//! let input = FlakeRef::parse("./vendor/foo", Context::FlakeInput).unwrap();
//! assert!(matches!(input.kind(), FlakeRefType::Path(_)));
//!
//! // A pure flake reference would error on a relative path.
//! assert!(FlakeRef::parse("./vendor/foo", Context::Standalone).is_err());
//!
//! let (flake_ref, attr) = FlakeRef::parse_with_fragment("nixpkgs#hello").unwrap();
//! assert_eq!(flake_ref.to_string(), "flake:nixpkgs");
//! assert_eq!(attr.as_deref(), Some("hello"));
//! ```

mod context;
mod error;
mod flakeref;
pub(crate) mod parser;

pub use context::Context;
pub use error::{NixUriError, NixUriResult, UnsupportedReason};
pub use flakeref::{
    AttrValue, Attrs, Download, FlakeRef, FlakeRefType, ForgeIdentity, GitForge, GitForgePlatform,
    GitRepo, Indirect, MercurialRepo, PathRef, RefKind,
};
