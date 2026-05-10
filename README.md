# nix-uri
[![Crates](https://img.shields.io/crates/v/nix-uri?style=flat-square)](https://crates.io/crates/nix-uri)
[![Documentation](https://img.shields.io/badge/nix_uri-documentation-fc0060?style=flat-square)](https://docs.rs/nix-uri)
[![Matrix Chat Room](https://img.shields.io/badge/chat-on%20matrix-1d7e64?logo=matrix&style=flat-square)](https://matrix.to/#/#nix-uri:matrix.org)


<!-- cargo-rdme start -->


[nix-uri](https://crates.io/crates/nix-uri) is a rust crate that parses
the [nix-uri-scheme](https://nixos.org/manual/nix/stable/command-ref/new-cli/nix3-flake#url-like-syntax)
into a [`flakeref::FlakeRef`] struct.

Also allows for building a `nix-uri` through the [`flakeref::FlakeRef`]
struct.

Convenience functionality for working with nix `flake.nix` references (flakerefs).
Provides types for the generic attribute set representation, but does not parse it:

``` markdown
   {
     type = "github";
     owner = "NixOS";
     repo = "nixpkgs";
   }
```

### Installation

To use `nix-uri`, add it as a dependency in your `Cargo.toml` file:

```markdown
[dependencies]
nix-uri = "0.1.10"
```

or use `cargo add`:

```markdown
cargo add nix-uri
```

## Examples
Check out the examples directory, for more information, or run an example:

```markdown
cargo run --example simple
cargo run --example cli github:nixos/nixpkgs
```

The uri syntax representation is parsed by this library:
### Example: Parsing from `github:nixos/nixpkgs`:

 ```rust
  let uri = "github:nixos/nixpkgs";
  let parsed: FlakeRef = uri.parse().unwrap();
  match parsed.kind {
      FlakeRefType::GitForge(forge) => {
          assert_eq!(forge.platform, GitForgePlatform::GitHub);
          assert_eq!(forge.owner, "nixos");
          assert_eq!(forge.repo, "nixpkgs");
          assert!(forge.ref_.is_none() && forge.rev.is_none());
      }
      _ => panic!("expected GitForge"),
  }
  ```

  The `Display` round-trip preserves the original form:
  ### Example: Round-tripping `github:nixos/nixpkgs`:
  ```rust
  let uri = "github:nixos/nixpkgs";
  let parsed: FlakeRef = uri.parse().unwrap();
  assert_eq!(uri, parsed.to_string());
  ```

<!-- cargo-rdme end -->

# Note 

This library is still an early WIP and not all cases are covered yet.
Especially error handling is not properly implemented at this time.
