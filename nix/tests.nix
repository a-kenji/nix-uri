_: {
  perSystem =
    {
      pkgs,
      self',
      ...
    }:
    {
      legacyPackages.fuzz-cli = pkgs.nixosTest {
        name = "fuzz-cli";
        nodes.machine =
          { ... }:
          {
            imports = [
              {
                environment.systemPackages = [ self'.checks.fuzz ];
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
    };
}
