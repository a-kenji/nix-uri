{
  self,
  nixpkgs,
  rust-overlay,
  flake-utils,
  flake-compat,
}:
flake-utils.lib.eachSystem [
  "aarch64-linux"
  "aarch64-darwin"
  "i686-linux"
  "x86_64-darwin"
  "x86_64-linux"
]
(system: let
  overlays = [(import rust-overlay)];

  pkgs = import nixpkgs {inherit system overlays;};

  name = "fixme";
  pname = name;
  root = self;

  ignoreSource = [".git" "target" "example"];

  src = pkgs.nix-gitignore.gitignoreSource ignoreSource root;

  cargoToml = builtins.fromTOML (builtins.readFile (src + ./Cargo.toml));
  rustToolchainToml = pkgs.rust-bin.fromRustupToolchainFile (src + "/rust-toolchain.toml");

  cargoLock = {
    lockFile = builtins.path {
      path = src + "/Cargo.lock";
      name = "Cargo.lock";
    };
  };
  cargo = rustToolchainToml;
  rustc = rustToolchainToml;

  buildInputs = [
  ];
  nativeBuildInputs = [
  ];
  devInputs = [
    rustToolchainToml

    pkgs.rust-analyzer

    pkgs.just

  ];
  fmtInputs = [
    pkgs.alejandra
    pkgs.treefmt
  ];
  ciInputs = [
    pkgs.typos
    pkgs.reuse
    pkgs.cargo-deny
  ];

  meta = with pkgs.lib; {
    homepage = "https://github.com/a-kenji/zellij-nix/";
    description = "A lsp implementation for nix";
    license = [licenses.agpl3];
  };
in rec {
  packages.default = (pkgs.makeRustPlatform {inherit cargo rustc;}).buildRustPackage {
    inherit
      src
      name
      cargoLock
      buildInputs
      nativeBuildInputs
      meta
      ;
  };
  # nix run
  apps.default = flake-utils.lib.mkApp {drv = packages.default;};

  devShells = {
    default = pkgs.callPackage ./devShell.nix {
      inherit buildInputs;
      nativeBuildInputs = nativeBuildInputs ++ devInputs ++ fmtInputs ++ ciInputs;
    };
    fmtShell = pkgs.mkShell {
      name = "fmt-shell";
      nativeBuildInputs = fmtInputs;
    };
    ciShell = pkgs.mkShell {
      name = "ci-shell";
      nativeBuildInputs = ciInputs;
    };
  };
})
// rec {
  overlays = {
    default = final: prev: rec {
      nix-analyzer = self.packages.${prev.system}.nix-analyzer;
    };
    nightly = final: prev: rec {
      nix-analyzer = self.packages.${prev.system}.nix-analyzer;
    };
  };
}
