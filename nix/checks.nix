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
              nu scripts/tests/e2e-keys.nu
              nu scripts/tests/e2e-recording.nu
              nu scripts/tests/e2e-state.nu
              nu scripts/tests/e2e-file-fixtures.nu
              nu scripts/tests/e2e-script-fixtures.nu
              nu scripts/tests/latency.nu
              nu scripts/tests/update-rune.nu
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
          inherit (dupin) bundledExtensions;
        };
        # The scope keys the Home Manager module checks denials against, read by Nix from each
        # bundled package.json, against what Dupin itself reads from the packages it ships.
        bundled-extension-scopes =
          pkgs.runCommand "dupin-bundled-extension-scopes"
            {
              nativeBuildInputs = [ pkgs.jq ];
              expected = builtins.toJSON dupin.bundledExtensions;
              passAsFile = [ "expected" ];
            }
            ''
              export HOME=$TMPDIR
              for package in ${dupin}/Applications/Dupin.app/Contents/Resources/extensions/*/; do
                ${lib.getExe' dupin "dupin"} ext revision "$package" --json \
                  | jq -c '{(.package): .scopes}'
              done | jq -s --sort-keys 'add | map_values(sort)' > actual.json
              jq --sort-keys 'map_values(sort)' "$expectedPath" > expected.json
              diff expected.json actual.json
              touch $out
            '';
        treefmt-typos-report-only =
          pkgs.runCommand "dupin-treefmt-typos-report-only"
            { nativeBuildInputs = [ config.treefmt.build.wrapper ]; }
            ''
              export HOME=$TMPDIR
              mkdir repo
              cd repo
              touch flake.nix
              cp ${root + "/typos.toml"} typos.toml
              text='teh behaviour' # spellchecker:disable-line
              printf '%s\n' "$text" > notes.txt
              if treefmt --no-cache notes.txt; then
                echo "treefmt accepted a typo instead of reporting it" >&2
                exit 1
              fi
              if [ "$(cat notes.txt)" != "$text" ]; then
                echo "typos rewrote notes.txt: $(cat notes.txt)" >&2
                exit 1
              fi
              touch $out
            '';
        renovate = repoCheck "dupin-renovate" [ ] "nu scripts/tests/renovate.nu";
        # The bundled extensions and the notes example against sdk/dupin.d.ts, as their authors'
        # editors see them.
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
