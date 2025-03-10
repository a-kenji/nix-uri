// #![forbid(unsafe_code)]
// #![warn(clippy::pedantic)]
// #![warn(clippy::nursery)]
// #![warn(clippy::cargo, unused)]
// #![allow(clippy::module_name_repetitions)]
// #![allow(clippy::uninlined_format_args)]
// #![allow(clippy::missing_errors_doc)]
// #![allow(clippy::must_use_candidate)]
// #![allow(clippy::no_effect_underscore_binding)]
//!
//! [nix-uri](https://crates.io/crates/nix-uri) is a rust crate that parses
//! the [nix-uri-scheme](https://nixos.org/manual/nix/stable/command-ref/new-cli/nix3-flake#url-like-syntax)
//! into a [`FlakeRef`](flakeref::FlakeRef) struct.
//!
//! Also allows for building a `nix-uri` through the [`FlakeRef`](flakeref::FlakeRef)
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
//! nix-uri = "0.1.9"
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
//!   # use nix_uri::FlakeRef;
//!   # use nix_uri::FlakeRefType;
//!   # use nix_uri::GitForgePlatform;
//!   # use nix_uri::GitForge;
//!   let uri = "github:nixos/nixpkgs";
//!   let expected = FlakeRef::new(
//!                 FlakeRefType::GitForge (GitForge{
//!                 platform: GitForgePlatform::GitHub,
//!                 owner: "nixos".into(),
//!                 repo: "nixpkgs".into(),
//!                 ref_or_rev: None,
//!                 }));
//!      let parsed: FlakeRef = uri.parse().unwrap();
//!      assert_eq!(expected, parsed);
//!   ```
//!
//!   It can also be generated from [`FlakeRef`](flakeref::Flakeref).
//!   ## Example: Constructing the following uri `github:nixos/nixpkgs`:
//!   ```
//!   # use nix_uri::FlakeRef;
//!   # use nix_uri::FlakeRefType;
//!   # use nix_uri::GitForgePlatform;
//!   # use nix_uri::GitForge;
//!   let expected = "github:nixos/nixpkgs";
//!   let uri = FlakeRef::new(
//!                 FlakeRefType::GitForge (GitForge{
//!                 platform: GitForgePlatform::GitHub,
//!                 owner: "nixos".into(),
//!                 repo: "nixpkgs".into(),
//!                 ref_or_rev: None,
//!                 })).to_string();
//!      assert_eq!(expected, uri);
//!   ```

// TODO: remove from error
mod error;
mod flakeref;
pub(crate) mod parser;
pub mod urls;

pub use error::*;
pub use flakeref::*;
