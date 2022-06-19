{
  mkShell,
  buildInputs,
  nativeBuildInputs,
}:
mkShell {
  name = "nxlsp-dev-env";
  inherit buildInputs nativeBuildInputs;
  ### Environment Variables
  RUST_BACKTRACE = 1;
}
