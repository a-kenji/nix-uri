{
  self,
  pkgs,
  ...
}:
let
  RUST_TOOLCHAIN = self + "/rust-toolchain.toml";
in
rec {
  rustToolchainTOML = pkgs.rust-bin.fromRustupToolchainFile RUST_TOOLCHAIN;
  rustLatestNightlyToolchain = pkgs.rust-bin.selectLatestNightlyWith (toolchain: toolchain.default);
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
