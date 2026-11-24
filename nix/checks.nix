{
  lib,
  self,
  inputs,
  ...
}:
let
  root = ./..;
in
{
  perSystem =
    { config, pkgs, ... }:
    let
      inherit (config.packages) dupin;
      # A check that runs the Nushell `script` over the repository's tracked files.
      repoCheck =
        name: inputs: script:
        pkgs.runCommand name { nativeBuildInputs = [ pkgs.nushell ] ++ inputs; } ''
          cd ${root}
          nu --no-config-file --commands ${lib.escapeShellArg script}
          touch $out
        '';
    in
    {
      checks = {
        inherit dupin;
        e2e-assets =
          repoCheck "dupin-e2e-assets" [ pkgs.coreutils pkgs.jq pkgs.bash pkgs.zip pkgs.unzip ]
            ''
              nu scripts/tests/e2e-assets.nu
              nu scripts/tests/e2e-comment.nu
              nu scripts/tests/e2e-recording.nu
              nu scripts/tests/e2e-state.nu
              nu scripts/tests/e2e-file-fixtures.nu
              nu scripts/tests/e2e-script-fixtures.nu
              nu scripts/tests/latency.nu
            '';
        inherit (dupin) hawk;
        # Clippy and the tests share one debug build of the dependencies. Formatting is
        # `checks.treefmt`, from nix/treefmt.nix.
        lint-and-test = dupin.lintAndTest;
        dependency-cache-keys = import ./tests/dependency-cache-keys.nix {
          inherit pkgs;
          inherit (inputs) crane;
          native = dupin.baseline;
          inherit (config.packages) cargo-hawk rolldown;
        };
        home-manager-module = import ./tests/home-manager-module.nix {
          inherit lib pkgs;
          module = self.homeManagerModules.default;
        };
        renovate = repoCheck "dupin-renovate" [ ] "nu scripts/tests/renovate.nu";
        # The bundled extensions against sdk/dupin.d.ts, as their authors' editors see them.
        extensions-typecheck = repoCheck "dupin-extensions-typecheck" [ pkgs.typescript ] ''
          ^tsc --project extensions/tsconfig.json
        '';
        gitleaks = repoCheck "dupin-gitleaks" [ pkgs.gitleaks ] ''
          ^gitleaks detect --source . --config .gitleaks.toml --no-git --redact
        '';
        nushell-ide-check = repoCheck "dupin-nushell-ide-check" [ ] ''
          let errors = glob scripts/**/*.nu | each {|script|
            nu --no-config-file --ide-check 10 $script | complete | get stdout | lines | each { from json }
            | where severity? == "Error" | insert file $script
          } | flatten
          if ($errors | is-not-empty) {
            error make {msg: ($errors | select file message | to text)}
          }
        '';
      };
    };
}
