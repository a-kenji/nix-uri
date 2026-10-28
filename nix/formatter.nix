{ inputs, ... }:
{
  imports = [ inputs.treefmt-nix.flakeModule ];

  perSystem = _: {
    treefmt = {
      programs.nixfmt.enable = true;
      programs.rustfmt.enable = true;
      programs.taplo.enable = true;
      programs.yamlfmt.enable = true;

      settings.formatter.rustfmt.options = [
        "--config"
        "newline_style=Unix"
      ];
    };
  };
}
