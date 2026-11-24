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
              package = pkgs.emptyDirectory;
            }
            // dupin;
          };
        }
      ];
    }).config;
  fixture = ./hm-extension;
  plain = evaluate { settings.hotkey = "alt+space"; };
  declared = evaluate {
    launchd.enable = true;
    extensions = [
      {
        src = fixture;
        grants = [ "clipboard:write" ];
      }
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
assert declared.xdg.configFile."dupin/extensions/word-count-copy".source == fixture;
assert declared.launchd.agents.dupin.enable;
assert !failed declared && failed duplicated;
pkgs.runCommand "dupin-home-manager-module"
  {
    nativeBuildInputs = [
      pkgs.remarshal
      pkgs.jq
    ];
    grants = declared.xdg.configFile."dupin/extension-grants.toml".source;
    expected = builtins.toJSON {
      word-count.grants = [ "clipboard:write" ];
      word-count-copy.grants = [ ];
    };
  }
  ''
    toml2json "$grants" | jq --sort-keys . > actual.json
    printf '%s' "$expected" | jq --sort-keys . > expected.json
    diff expected.json actual.json
    touch $out
  ''
