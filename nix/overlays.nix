{
self,
...
}:
{
perSystem = { system, ... }: {
  _module.args.pkgs = import self.inputs.nixpkgs {
    inherit system;
    overlays = [
    (import self.inputs.rust-overlay)
    ];
    config = { };
  };
};
}
