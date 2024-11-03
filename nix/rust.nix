{
  self,
  pkgs,
  ...
}:
let
  RUST_TOOLCHAIN = self + "/rust-toolchain.toml";
  RUST_NIGHTLY_TOOLCHAIN = self + "/.rust-nightly-toolchain.toml";
in
rec {
  rustToolchainTOML = pkgs.rust-bin.fromRustupToolchainFile RUST_TOOLCHAIN;
  rustNightlyToolchainTOML = pkgs.rust-bin.fromRustupToolchainFile RUST_NIGHTLY_TOOLCHAIN;
  rustToolchainDevTOML = rustToolchainTOML.override {
    extensions = [
      "clippy"
      "rust-analysis"
      "rust-docs"
      "rust-src"
      "rustfmt"
    ];
  };
}
