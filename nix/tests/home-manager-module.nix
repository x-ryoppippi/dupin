# Evaluates the Home Manager module against stand-ins for the Home Manager options it sets, so
# the check needs no Home Manager input.
{
  lib,
  pkgs,
  module,
  # The scopes the real package's bundled packages declare, by their package.json names.
  bundledExtensions,
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
              package =
                pkgs.writeShellScriptBin "dupin" ''
                  [ "$1 $2 $4" = "ext revision --json" ] \
                    && printf '{"package": %s, "revision": "blake3:%s"}\n' \
                      "$(${lib.getExe pkgs.jq} .name "$3/package.json")" "$(basename "$3" | cut -c1-8)"
                ''
                // {
                  inherit bundledExtensions;
                };
            }
            // dupin;
          };
        }
      ];
    }).config;
  fixture = ./hm-extension;
  notes = ./hm-extension-notes;
  plain = evaluate { settings.hotkey = "alt+space"; };
  # `[advanced]` is part of `settings`, with arrays of tables for a kind's actions.
  advanced = evaluate {
    settings.advanced = {
      sources.apps.matcher = "builtin.prefix";
      commands.default.append_sources = [ "files" ];
      kinds.file.actions = [
        {
          title = "Open in Zed";
          script = "open-in-zed";
        }
      ];
    };
  };
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
    bundledExtensions = {
      window-management.enable = true;
      calendar.deny = [ "calendar" ];
      clipboard-history.enable = false;
      dictionary = { };
    };
  };
  bundledOnly = evaluate { bundledExtensions.window-management.enable = true; };
  unbundled = evaluate { bundledExtensions.word-count.enable = true; };
  undeclared = evaluate { bundledExtensions.calendar.deny = [ "clipboard:read" ]; };
  # A scope the broker can't read is a type error, whichever option names it.
  rejects =
    dupin:
    let
      inherit ((evaluate dupin).programs.dupin) extensions bundledExtensions;
    in
    !(builtins.tryEval (builtins.deepSeq [ extensions bundledExtensions ] true)).success;
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
assert bundledExtensions.calendar == [ "calendar" ];
assert
  bundledExtensions.window-management == [
    "windows:read"
    "windows:arrange"
  ];
assert lib.elem "clipboard:watch" bundledExtensions.clipboard-history;
assert bundledExtensions.dupin == [ ];
assert !failed bundledOnly && failed unbundled && failed undeclared;
assert lib.hasInfix "it declares calendar"
  (lib.findFirst (a: !a.assertion) null undeclared.assertions).message;
assert rejects { bundledExtensions.calendar.deny = [ "calender" ]; };
assert rejects {
  extensions = [
    {
      src = fixture;
      deny = [ "clipboard:copy" ];
    }
  ];
};
assert rejects {
  extensions = [
    {
      src = fixture;
      grants = [ "http:http://api.example.com" ];
    }
  ];
};
assert
  !rejects {
    extensions = [
      {
        src = fixture;
        grants = [
          "http:https://api.example.com:8443"
          "files:notes"
          "script:build.v2"
          "storage"
          "rows:file"
          "query"
        ];
      }
    ];
  };
pkgs.runCommand "dupin-home-manager-module"
  {
    nativeBuildInputs = [
      pkgs.remarshal
      pkgs.jq
    ];
    grants = declared.xdg.configFile."dupin/extension-grants.toml".source;
    advanced = advanced.xdg.configFile."dupin/config.toml".source;
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
        "bundled:window-management".enabled = true;
        "bundled:calendar".deny = [ "calendar" ];
        "bundled:clipboard-history".enabled = false;
      };
    };
    bundledGrants = bundledOnly.xdg.configFile."dupin/extension-grants.toml".source;
    bundledExpected = builtins.toJSON {
      version = 1;
      packages."bundled:window-management".enabled = true;
    };
  }
  ''
    toml2json "$advanced" | jq -e '.advanced == {
      sources: { apps: { matcher: "builtin.prefix" } },
      commands: { default: { append_sources: ["files"] } },
      kinds: { file: { actions: [{ title: "Open in Zed", script: "open-in-zed" }] } }
    }'
    toml2json "$bundledGrants" | jq --sort-keys . > bundled-actual.json
    printf '%s' "$bundledExpected" | jq --sort-keys . > bundled-expected.json
    diff bundled-expected.json bundled-actual.json
    toml2json "$grants" | jq --sort-keys 'del(.packages[].revision)' > actual.json
    toml2json "$grants" | jq -e '.packages | with_entries(select(.key | startswith("bundled:") | not)) | map(.revision) | all(startswith("blake3:"))'
    printf '%s' "$expected" | jq --sort-keys . > expected.json
    diff expected.json actual.json
    grep -F 'more than one package is named word-count in package.json' "$clash/testBuildFailure.log"
    touch $out
  ''
