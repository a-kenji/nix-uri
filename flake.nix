{
  description = "nix-uri - parse the nix-uri scheme.";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    crane.url = "github:ipetkov/crane";

    flake-utils.url = "github:numtide/flake-utils/";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    rust-overlay,
    crane,
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = nixpkgs.legacyPackages.${system};
        stdenv =
          if pkgs.stdenv.isLinux
          then pkgs.stdenvAdapters.useMoldLinker pkgs.stdenv
          else pkgs.stdenv;
        overlays = [(import rust-overlay)];
        rustPkgs = import nixpkgs {inherit system overlays;};
        src = self;
        RUST_TOOLCHAIN = src + "/rust-toolchain.toml";
        RUSTFMT_TOOLCHAIN = src + "/.rustfmt-toolchain.toml";
        cargoTOML = builtins.fromTOML (builtins.readFile (src + "/Cargo.toml"));
        inherit (cargoTOML.package) version name;
        rustToolchainTOML = rustPkgs.rust-bin.fromRustupToolchainFile RUST_TOOLCHAIN;
        rustFmtToolchainTOML =
          rustPkgs.rust-bin.fromRustupToolchainFile
          RUSTFMT_TOOLCHAIN;
        rustToolchainDevTOML = rustToolchainTOML.override {
          extensions = [
            "clippy"
            "rust-analysis"
            "rust-docs"
          ];
        };
        cargoLock = {
          lockFile = builtins.path {
            path = self + "/Cargo.lock";
            name = "Cargo.lock";
          };
          allowBuiltinFetchGit = true;
        };
        rustc = rustToolchainTOML;
        cargo = rustToolchainTOML;

        devInputs = [
          rustToolchainDevTOML
          rustFmtToolchainTOML
          pkgs.just

          pkgs.cargo-fuzz
          pkgs.cargo-flamegraph
          pkgs.cargo-diet
          pkgs.cargo-tarpaulin
          pkgs.cargo-public-api

          (pkgs.symlinkJoin {
            name = "cargo-udeps-wrapped";
            paths = [pkgs.cargo-udeps];
            nativeBuildInputs = [pkgs.makeWrapper];
            postBuild = ''
              wrapProgram $out/bin/cargo-udeps \
                --prefix PATH : ${
                pkgs.lib.makeBinPath [
                  (rustPkgs.rust-bin.selectLatestNightlyWith (toolchain: toolchain.default))
                ]
              }
            '';
          })
          (pkgs.symlinkJoin {
            name = "cargo-careful-wrapped";
            paths = [pkgs.cargo-careful];
            nativeBuildInputs = [pkgs.makeWrapper];
            postBuild = ''
              wrapProgram $out/bin/cargo-careful \
                --prefix PATH : ${
                pkgs.lib.makeBinPath [
                  (rustPkgs.rust-bin.selectLatestNightlyWith (
                    toolchain: toolchain.default.override {extensions = ["rust-src"];}
                  ))
                ]
              }
            '';
          })
          pkgs.cargo-rdme

          pkgs.llvmPackages.bintools
          pkgs.mold
          pkgs.clang
        ];
        shellInputs = [
          pkgs.shellcheck
          pkgs.actionlint
        ];
        fuzzInputs = [
          pkgs.cargo-fuzz
          rustFmtToolchainTOML
        ];
        fmtInputs = [
          rustFmtToolchainTOML
          pkgs.alejandra
          pkgs.treefmt
          pkgs.taplo
          pkgs.typos
        ];
        editorConfigInputs = [pkgs.editorconfig-checker];
        actionlintInputs = [pkgs.actionlint];
        targetDir = "target/${
          pkgs.rust.toRustTarget pkgs.stdenv.targetPlatform
        }/release";
        commonArgs = {
          inherit
            src
            stdenv
            version
            name
            ;
          pname = name;
        };
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchainTOML;
        mkExample = {example, ...}:
          craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts stdenv;
              pname = example;
              cargoExtraArgs = "--example ${example}";
              doCheck = false;
            }
          );

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        cargoDoc = craneLib.cargoDoc (commonArgs // {inherit cargoArtifacts;});
        cargoClippy = craneLib.cargoClippy (
          commonArgs
          // {
            inherit cargoArtifacts;
            nativeBuildInputs = [rustToolchainDevTOML];
          }
        );
        cargoTarpaulin = craneLib.cargoTarpaulin (
          commonArgs // {inherit cargoArtifacts;}
        );
        cargoLlvmCov = craneLib.cargoLlvmCov (
          commonArgs // {inherit cargoArtifacts;}
        );
        cargoTest = craneLib.cargoTest (
          commonArgs // {inherit cargoArtifacts;}
        );
      in rec {
        devShells = {
          default = (pkgs.mkShell.override {inherit stdenv;}) {
            buildInputs =
              shellInputs ++ fmtInputs ++ devInputs;
            inherit name;
            FLK_LOG = "debug";
            RUST_BACKTRACE = true;
            RUSTFLAGS = "-C linker=clang -C link-arg=-fuse-ld=${pkgs.mold}/bin/mold";
          };
          editorConfigShell = pkgs.mkShell {buildInputs = editorConfigInputs;};
          actionlintShell = pkgs.mkShell {buildInputs = actionlintInputs;};
          fmtShell = pkgs.mkShell {buildInputs = fmtInputs;};
          fuzzShell = pkgs.mkShell {buildInputs = fuzzInputs;};
        };
        packages =
          {
            default = packages.crane;
            upstream = (pkgs.makeRustPlatform {inherit cargo rustc;}).buildRustPackage {
              cargoDepsName = name;
              doCheck = false;
              ASSET_DIR = "${targetDir}/assets/";
              inherit
                name
                src
                stdenv
                cargoLock
                ;
            };
            crane = craneLib.buildPackage (
              commonArgs
              // {
                cargoExtraArgs = "-p ${name}";
                doCheck = false;
                pname = name;
                inherit name cargoArtifacts stdenv;
              }
            );
            fuzz =
              ((crane.mkLib pkgs).overrideToolchain rustFmtToolchainTOML).buildPackage
              rec {
                src = ./.;
                cargoArtifacts =
                  ((crane.mkLib pkgs).overrideToolchain rustFmtToolchainTOML).buildDepsOnly
                  {src = ./.;};
                __flags = [
                  "--cfg fuzzing"
                  "--cfg fuzzing_repro"
                  "-Cpasses=sancov-module"
                  "-C opt-level=3"
                  "-Cllvm-args=-sanitizer-coverage-level=4"
                  "-Cllvm-args=-sanitizer-coverage-inline-8bit-counters"
                  "-Cllvm-args=-sanitizer-coverage-pc-table"
                  "-Cllvm-args=-sanitizer-coverage-trace-compares"
                  "-Z sanitizer=address"
                  "-Zsanitizer=memory" # memory 1
                  "-Zsanitizer-memory-track-origins" # memory 2
                  "-Cllvm-args=-sanitizer-coverage-stack-depth" # only linux
                  "-Cdebug-assertions"
                ];
                buildFlags = __flags;
                cargoBuildCommand = "cargo b --package=nix-uri-fuzz --bin fuzz_comp_err";
                CARGO_PROFILE = "fuzz";
                doCheck = false;
                pname = "fuzz_comp_err";
                nativeBuildInputs = fuzzInputs;
              };
            inherit
              cargoArtifacts
              cargoClippy
              cargoDoc
              cargoTarpaulin
              cargoLlvmCov
              cargoTest
              ;
          }
          // pkgs.lib.genAttrs ["cli"] (
            example: mkExample {inherit example cargoArtifacts craneLib;}
          );

        checks = {
          inherit
            cargoArtifacts
            cargoClippy
            cargoDoc
            cargoTest
            ;
        };
        formatter = pkgs.alejandra;
        nixosModules.fuzz-cli = pkgs.nixosTest {
          name = "fuzz-cli";
          nodes.machine = {...}: {
            imports = [
              {
                environment.systemPackages = [self.outputs.packages.x86_64-linux.fuzz];
                virtualisation.graphics = false;
                documentation.enable = false;
              }
            ];
          };
          testScript = ''
            start_all()
            with subtest("fuzzing"):
                stdout = machine.succeed("fuzz_comp_err", timeout=None)
                print(stdout)
            machine.shutdown()
          '';
        };
      }
    );
}
