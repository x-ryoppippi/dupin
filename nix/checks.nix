{ lib, ... }:
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
        # Clippy and the tests share one debug build of the dependencies. Formatting is
        # `checks.treefmt`, from nix/treefmt.nix.
        lint-and-test = dupin.lintAndTest;
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
