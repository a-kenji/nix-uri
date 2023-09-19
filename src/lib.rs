//! #![forbid(unsafe_code)]
//! #![warn(clippy::pedantic, clippy::nursery, clippy::cargo, unused)]
//!
//! [nix-uri](https://crates.io/crates/nix-uri) is a rust crate that parses
//! the [nix-uri-scheme](https://nixos.org/manual/nix/stable/command-ref/new-cli/nix3-flake#url-like-syntax)
//! into a [`FlakeRef`](flakeref::FlakeRef) struct.
//!
//! Also allows for building a `nix-uri` through the [`FlakeRef`](flakeref::FlakeRef) struct.
//!
//! Convenience functionality for working with nix `flake.nix` references (flakerefs).
//! Provides types for the generic attribute set representation, but does not parse it:
//!
//! ```no_run
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
//! ```
//! [dependencies]
//! nix-uri = "0.1.4"
//! ```
//!
//! or use `cargo add`:
//!
//! ```
//! cargo add nix-uri
//! ```
//!
//! # Examples
//! Check out the examples directory, for more information, or run an example:
//!
//! ```
//! cargo run --example simple
//! cargo run --example cli github:nixpkgs/nixos
//! ```
//!
//! The uri syntax representation is parsed by this library:
//! ## Example: Parsing from `github:nixos/nixpkgs`:
//!
//!  ```
//!   # use nix_uri::FlakeRef;
//!   # use nix_uri::FlakeRefType;
//!   let uri = "github:nixos/nixpkgs";
//!   let expected = FlakeRef::new(
//!                 FlakeRefType::GitHub {
//!                 owner: "nixos".into(),
//!                 repo: "nixpkgs".into(),
//!                 ref_or_rev: None,
//!                 });
//!      let parsed: FlakeRef = uri.try_into().unwrap();
//!      assert_eq!(expected, parsed);
//!   ```
//!
//!   It can also be generated from [`FlakeRef`](flakeref::Flakeref).
//!   ## Example: Constructing the following uri `github:nixos/nixpkgs`:
//!   ```
//!   # use nix_uri::FlakeRef;
//!   # use nix_uri::FlakeRefType;
//!   let expected = "github:nixos/nixpkgs";
//!   let uri = FlakeRef::new(
//!                 FlakeRefType::GitHub {
//!                 owner: "nixos".into(),
//!                 repo: "nixpkgs".into(),
//!                 ref_or_rev: None,
//!                 }).to_string();
//!      assert_eq!(expected, uri);
//!   ```

// TODO: remove from error
#[allow(unused)]
mod error;
#[allow(unused)]
mod flakeref;
mod parser;
#[cfg(feature = "url")]
mod url;

pub use error::*;
pub use flakeref::*;
