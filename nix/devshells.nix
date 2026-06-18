{ self, ... }:
{
  perSystem =
    {
      pkgs,
      self',
      ...
    }:
    let
      env = import ./env.nix { inherit pkgs; };
      rust = pkgs.callPackage ./rust.nix { inherit self; };
    in
    {
      devShells = {
        default = pkgs.mkShellNoCC {
          name = "nix-uri";
          inputsFrom = [ self'.packages.default ];
          packages = [
            rust.rustToolchainDevTOML
            pkgs.just
            pkgs.rust-analyzer
            self'.formatter.outPath
          ];
          inherit env;
        };
        cliff = pkgs.mkShellNoCC {
          name = "changelog";
          packages = [
            pkgs.git-cliff
          ];
        };
        full = pkgs.mkShellNoCC {
          name = "nix-uri-full";
          inputsFrom = [ self'.devShells.default ];
          packages = [
            pkgs.cargo-deny
            pkgs.cargo-mutants
            pkgs.cargo-tarpaulin
            pkgs.cargo-public-api
            pkgs.cargo-rdme
          ];
          inherit env;
        };
        fuzz = pkgs.mkShellNoCC {
          name = "nix-uri-nightly-fuzz";
          inputsFrom = [ self'.devShells.default ];
          packages = [
            rust.rustLatestNightlyToolchain
            pkgs.cargo-fuzz
          ];
          inherit env;
        };
      };
    };
}
