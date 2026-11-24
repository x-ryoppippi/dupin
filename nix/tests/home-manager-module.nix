# Evaluates the Home Manager module against stand-ins for the Home Manager options it sets, so
# the check needs no Home Manager input.
{
  lib,
  pkgs,
  module,
}:
let
  evaluate =
    dupin:
    (lib.evalModules {
      modules = [
        module
        {
          options = {
            assertions = lib.mkOption {
              type = lib.types.listOf lib.types.unspecified;
              default = [ ];
            };
            home.packages = lib.mkOption {
              type = lib.types.listOf lib.types.package;
              default = [ ];
            };
            xdg = {
              enable = lib.mkEnableOption "xdg";
              configHome = lib.mkOption { default = "/home/config"; };
              dataHome = lib.mkOption { default = "/home/data"; };
              configFile = lib.mkOption {
                type = lib.types.attrsOf (
                  lib.types.submodule { options.source = lib.mkOption { type = lib.types.path; }; }
                );
                default = { };
              };
            };
            launchd.agents = lib.mkOption {
              type = lib.types.attrsOf lib.types.unspecified;
              default = { };
            };
          };
          config = {
            _module.args.pkgs = pkgs;
            programs.dupin = {
              enable = true;
              # Stands in for `dupin ext revision --json`, which names the package in the folder
              # and its revision.
              package = pkgs.writeShellScriptBin "dupin" ''
                [ "$1 $2 $4" = "ext revision --json" ] \
                  && printf '{"package": %s, "revision": "blake3:%s"}\n' \
                    "$(${lib.getExe pkgs.jq} .name "$3/package.json")" "$(basename "$3" | cut -c1-8)"
              '';
            }
            // dupin;
          };
        }
      ];
    }).config;
  fixture = ./hm-extension;
  notes = ./hm-extension-notes;
  plain = evaluate { settings.hotkey = "alt+space"; };
  declared = evaluate {
    launchd.enable = true;
    extensions = [
      {
        src = fixture;
        grants = [
          "clipboard:write"
          "files:notes"
        ];
        bindings."files:notes" = "/Users/me/Notes";
      }
      {
        src = notes;
        name = "my-notes";
        deny = [ "clipboard:read" ];
        enable = false;
      }
    ];
  };
  # Two folders of one package, whose grants would both apply to it.
  clashing = evaluate {
    extensions = [
      { src = fixture; }
      {
        src = fixture;
        name = "word-count-copy";
      }
    ];
  };
  duplicated = evaluate {
    extensions = [
      { src = fixture; }
      { src = fixture; }
    ];
  };
  failed = config: lib.any (assertion: !assertion.assertion) config.assertions;
in
assert lib.attrNames plain.xdg.configFile == [ "dupin/config.toml" ];
assert declared.xdg.configFile."dupin/extensions/word-count".source == fixture;
assert declared.xdg.configFile."dupin/extensions/my-notes".source == notes;
assert declared.launchd.agents.dupin.enable;
assert !failed declared && failed duplicated;
pkgs.runCommand "dupin-home-manager-module"
  {
    nativeBuildInputs = [
      pkgs.remarshal
      pkgs.jq
    ];
    grants = declared.xdg.configFile."dupin/extension-grants.toml".source;
    clash = pkgs.testers.testBuildFailure clashing.xdg.configFile."dupin/extension-grants.toml".source;
    expected = builtins.toJSON {
      version = 1;
      packages = {
        word-count = {
          enabled = true;
          grant = [
            "clipboard:write"
            "files:notes"
          ];
          deny = [ ];
          bindings."files:notes" = "/Users/me/Notes";
        };
        notes = {
          enabled = false;
          grant = [ ];
          deny = [ "clipboard:read" ];
          bindings = { };
        };
      };
    };
  }
  ''
    toml2json "$grants" | jq --sort-keys 'del(.packages[].revision)' > actual.json
    toml2json "$grants" | jq -e '[.packages[].revision] | all(startswith("blake3:"))'
    printf '%s' "$expected" | jq --sort-keys . > expected.json
    diff expected.json actual.json
    grep -F 'more than one package is named word-count in package.json' "$clash/testBuildFailure.log"
    touch $out
  ''
