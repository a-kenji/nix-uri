{self, ...}: {
  perSystem =
    { pkgs, ... }:
    {
      checks = {
        inherit ((pkgs.callPackage ./crane.nix { inherit self; }))
          simple
          cli
          fuzz
          cargoArtifacts
          cargoClippy
          cargoDeny
          cargoDoc
          cargoTest
          cargoTarpaulin
          ;
      };
    };
}
