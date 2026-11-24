{
  lib,
  writeShellScriptBin,
  nodejs,
  rolldown,
}:
# nixpkgs packages Rolldown as a Node library without a `rolldown` on PATH.
writeShellScriptBin "rolldown" ''
  exec ${lib.getExe nodejs} ${rolldown}/lib/node_modules/rolldown/bin/cli.mjs "$@"
''
