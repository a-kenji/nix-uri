{
  self,
  lib,
  pkgs,
}:
let
  rust = pkgs.callPackage ./rust.nix { inherit self; };
  cargoTOML = builtins.fromTOML (builtins.readFile (self + "/Cargo.toml"));
  inherit (cargoTOML.package) version name;
  craneLib = (self.inputs.crane.mkLib pkgs).overrideToolchain rust.rustToolchainTOML;
  src = lib.cleanSourceWith { src = craneLib.path ../.; };
  commonArgs = {
    inherit
      src
      version
      name
      ;
    pname = name;
    cargoExtraArgs = "--locked --all-features";
  };
  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
  mkExample =
    { example, ... }:
    craneLib.buildPackage (
      commonArgs
      // {
        inherit cargoArtifacts;
        pname = "${example}-example-${name}";
        name = "${example}-example-${name}";
        cargoExtraArgs = "${commonArgs.cargoExtraArgs} --example ${example}";
        doCheck = false;
        meta.mainProgram = example;
      }
    );
  cargoClippy = craneLib.cargoClippy (
    commonArgs
    // {
      inherit cargoArtifacts;
      cargoClippyExtraArgs = "--all-targets -- --deny warnings";
      nativeBuildInputs = [ rust.rustToolchainDevTOML ];
    }
  );
  cargoDeny = craneLib.cargoDeny (commonArgs // { inherit cargoArtifacts; });
  cargoTarpaulin = craneLib.cargoTarpaulin (commonArgs // { inherit cargoArtifacts; });
  cargoDoc = craneLib.cargoDoc (commonArgs // { inherit cargoArtifacts; });
  cargoTest = craneLib.cargoNextest (commonArgs // { inherit cargoArtifacts; });
  craneLibNightly = (self.inputs.crane.mkLib pkgs).overrideToolchain rust.rustLatestNightlyToolchain;
  fuzzArgs = commonArgs // rec {
    pname = "fuzz_comp_err";
    name = pname;
    cargoExtraArgs = "--package=nix-uri-fuzz --bin fuzz_comp_err";
    CARGO_PROFILE = "fuzz";
    CARGO_BUILD_TARGET = pkgs.stdenv.hostPlatform.rust.rustcTarget;
    RUSTFLAGS = lib.concatStringsSep " " (
      [
        "--cfg fuzzing"
        "-Cpasses=sancov-module"
        "-Cllvm-args=-sanitizer-coverage-level=4"
        "-Cllvm-args=-sanitizer-coverage-inline-8bit-counters"
        "-Cllvm-args=-sanitizer-coverage-pc-table"
        "-Cllvm-args=-sanitizer-coverage-trace-compares"
        "-Zsanitizer=address"
        "-Cdebug-assertions"
        "-Coverflow-checks"
      ]
      ++ lib.optional pkgs.stdenv.hostPlatform.isLinux "-Cllvm-args=-sanitizer-coverage-stack-depth"
    );
    doCheck = false;
  };
  fuzz = craneLibNightly.buildPackage (
    fuzzArgs
    // {
      cargoArtifacts = craneLibNightly.buildDepsOnly fuzzArgs;
      meta.mainProgram = "fuzz_comp_err";
    }
  );
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
// pkgs.lib.genAttrs [
  "cli"
  "simple"
] (example: mkExample { inherit example cargoArtifacts craneLib; })
