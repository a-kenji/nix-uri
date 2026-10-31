# nix-uri
[![Crates](https://img.shields.io/crates/v/nix-uri?style=flat-square)](https://crates.io/crates/nix-uri)
[![Documentation](https://img.shields.io/badge/nix_uri-documentation-fc0060?style=flat-square)](https://docs.rs/nix-uri)
[![Matrix Chat Room](https://img.shields.io/badge/chat-on%20matrix-1d7e64?logo=matrix&style=flat-square)](https://matrix.to/#/#nix-uri:matrix.org)


<!-- cargo-rdme start -->


[nix-uri](https://crates.io/crates/nix-uri) parses [flake references](https://nixos.org/manual/nix/stable/command-ref/new-cli/nix3-flake#url-like-syntax) into a `FlakeRef`.

A `FlakeRef` carries Nix's attributes: `FlakeRef::to_attrs` and
`FlakeRef::from_attrs` reads a node of `flake.lock` or a registry entry.
Its `Display` tries to mirror how Nix would render these attributes, which means
they are not necessarily round-trip stable.

Example:

```rust
let flake_ref: FlakeRef = "github:NixOS/nixpkgs?dir=lib&ref=nixos-25.05".parse().unwrap();
assert_eq!(flake_ref.to_string(), "github:NixOS/nixpkgs/nixos-25.05?dir=lib");
```

### Installation

To use `nix-uri`, add it as a dependency in your `Cargo.toml` file:

```markdown
[dependencies]
nix-uri = "0.2.0"
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

### Example: Parsing `github:nixos/nixpkgs`

```rust
let parsed: FlakeRef = "github:nixos/nixpkgs".parse().unwrap();
match parsed.kind() {
    FlakeRefType::GitForge(forge) => {
        assert_eq!(forge.platform, GitForgePlatform::GitHub);
        assert_eq!(forge.owner, "nixos");
        assert_eq!(forge.repo, "nixpkgs");
        assert!(forge.ref_.is_none() && forge.rev.is_none());
    }
    _ => panic!("expected GitForge"),
}
```

### Parse context

Which flake references are accepted and how they are parsed depends on
the context they are in.
`FlakeRef::parse` and `Context`.
`FromStr` and `TryFrom` parse as `builtins.parseFlakeRef` would do
(`Context::Standalone`). If you want to parse flake references from the CLI,
they should be parsed through: `FlakeRef::parse_with_fragment`.

```rust
// `inputs.<name>.url` keeps relative paths.
let input = FlakeRef::parse("./vendor/foo", Context::FlakeInput).unwrap();
assert!(matches!(input.kind(), FlakeRefType::Path(_)));

// A pure flake reference would error on a relative path.
assert!(FlakeRef::parse("./vendor/foo", Context::Standalone).is_err());

let (flake_ref, attr) = FlakeRef::parse_with_fragment("nixpkgs#hello").unwrap();
assert_eq!(flake_ref.to_string(), "flake:nixpkgs");
assert_eq!(attr.as_deref(), Some("hello"));
```

<!-- cargo-rdme end -->

# Note 

This library is still an early WIP and not all cases are covered yet.
Especially error handling is not properly implemented at this time.
