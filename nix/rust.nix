{
  self,
  pkgs,
  ...
}: let
  RUST_TOOLCHAIN = self + "/rust-toolchain.toml";
  RUSTFMT_TOOLCHAIN = self + "/.rust-nightly-toolchain.toml";
in rec {
  rustToolchainTOML = pkgs.rust-bin.fromRustupToolchainFile RUST_TOOLCHAIN;
  rustNightlyToolchainTOML =
    pkgs.rust-bin.fromRustupToolchainFile
    RUSTFMT_TOOLCHAIN;
  rustToolchainDevTOML = rustToolchainTOML.override {
    extensions = [
      "clippy"
      "rust-src"
      "rust-analysis"
      "rust-docs"
    ];
  };
}
