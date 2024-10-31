_: {
  perSystem = {self', ...}: {
    packages = rec {
      default = cli;
      inherit
        (self'.checks)
        cli
        simple
        ;
    };
  };
}
