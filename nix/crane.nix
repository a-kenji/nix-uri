{
  self,
  lib,
  pkgs,
  cargo-fuzz,
}: let
  rust = pkgs.callPackage ./rust.nix {inherit self;};
  cargoTOML = builtins.fromTOML (builtins.readFile (self + "/Cargo.toml"));
  inherit (cargoTOML.package) version name;
  craneLib = (self.inputs.crane.mkLib pkgs).overrideToolchain rust.rustToolchainTOML;
  src = lib.cleanSourceWith {src = craneLib.path ../.;};
  commonArgs = {
    inherit
      src
      version
      name
      ;
    pname = name;
  };
  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
  mkExample = {example, ...}:
    craneLib.buildPackage (
      commonArgs
      // {
        inherit cargoArtifacts;
        pname = "${example}-example-${name}";
        name = "${example}-example-${name}";
        cargoExtraArgs = "--example ${example}";
        doCheck = false;
        meta.mainProgram = example;
      }
    );
  cargoClippy = craneLib.cargoClippy (
    commonArgs
    // {
      inherit cargoArtifacts;
      nativeBuildInputs = [rust.rustToolchainDevTOML];
    }
  );
  cargoDeny = craneLib.cargoDeny (commonArgs // {inherit cargoArtifacts;});
  cargoTarpaulin = craneLib.cargoTarpaulin (commonArgs // {inherit cargoArtifacts;});
  cargoDoc = craneLib.cargoDoc (commonArgs // {inherit cargoArtifacts;});
  cargoTest = craneLib.cargoNextest (commonArgs // {inherit cargoArtifacts;});
  fuzz = ((self.inputs.crane.mkLib pkgs).overrideToolchain rust.rustNightlyToolchainTOML).buildPackage (commonArgs
    // rec {
      inherit src;
      cargoArtifacts =
        ((self.inputs.crane.mkLib pkgs).overrideToolchain rust.rustNightlyToolchainTOML).buildDepsOnly
        commonArgs
        // {
          inherit version pname;
        };
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
      name = pname;
      nativeBuildInputs = [
        rust.rustNightlyToolchainTOML
        cargo-fuzz
      ];
      inherit version;
    });
in
  {
    inherit
      fuzz
      cargoClippy
      cargoArtifacts
      cargoDeny
      cargoTarpaulin
      cargoDoc
      cargoTest
      ;
  }
  // pkgs.lib.genAttrs ["cli" "simple"] (
    example: mkExample {inherit example cargoArtifacts craneLib;}
  )
