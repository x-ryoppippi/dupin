{ self, ... }:
{
  flake.homeManagerModules.default =
    {
      config,
      lib,
      pkgs,
      ...
    }:
    let
      cfg = config.programs.dupin;
      toml = pkgs.formats.toml { };
      # A bundled package runs only as the sealed copy that shipped, so its entry needs no
      # revision, and it has nothing to grant: Dupin pre-grants everything it declares.
      bundledPackages = lib.filterAttrs (_: package: package != { }) (
        lib.mapAttrs' (
          name: extension:
          lib.nameValuePair "bundled:${name}" (
            lib.optionalAttrs (extension.enable != null) { enabled = extension.enable; }
            // lib.optionalAttrs (extension.deny != [ ]) { inherit (extension) deny; }
          )
        ) cfg.bundledExtensions
      );
      bundled = cfg.package.bundledExtensions or { };
      bundledNames = lib.attrNames bundled;
      unknownBundled = lib.subtractLists bundledNames (lib.attrNames cfg.bundledExtensions);
      # A denial of a scope the package doesn't declare would deny nothing.
      undeclaredDenials = lib.concatLists (
        lib.mapAttrsToList (
          name: extension:
          let
            declared = bundled.${name} or [ ];
            undeclared = lib.subtractLists declared extension.deny;
          in
          lib.optional (lib.elem name bundledNames && undeclared != [ ])
            "${name} doesn't declare ${lib.concatStringsSep ", " undeclared}; it declares ${
              if declared == [ ] then "no scopes" else lib.concatStringsSep ", " declared
            }"
        ) cfg.bundledExtensions
      );
      # The scope keys the capability broker reads; an entry it can't read keeps its package off.
      scope =
        let
          host = "[a-z0-9]([a-z0-9-]*[a-z0-9])?";
          name = "[A-Za-z0-9][A-Za-z0-9._-]{0,63}";
        in
        lib.types.strMatching (
          "storage|dictionary|calculator|snippets|calendar|query|clipboard:(read|write|watch|paste)|windows:(read|arrange)|rows:(text|url|file|app)"
          + "|(files|exec|script):${name}|http:https://${host}(\\.${host})*(:[0-9]{1,5})?"
        );
      # The capability broker's declarative base. Dupin knows a package by `name` in its
      # package.json, whatever its folder is called, and its grants name the revision Dupin
      # computes from the package. Only Dupin computes both, so they are filled in when the file
      # builds rather than while Nix evaluates.
      grantsFile =
        pkgs.runCommand "dupin-extension-grants.toml"
          {
            nativeBuildInputs = [
              pkgs.jq
              pkgs.remarshal
            ];
            bundled = builtins.toJSON bundledPackages;
            packages = builtins.toJSON (
              map (extension: {
                inherit (extension)
                  src
                  grants
                  deny
                  bindings
                  ;
                enabled = extension.enable;
              }) cfg.extensions
            );
            passAsFile = [
              "packages"
              "bundled"
            ];
          }
          ''
            jq -c '.[]' "$packagesPath" | while read -r package; do
              src=$(jq -r .src <<<"$package")
              declared=$(${lib.getExe' cfg.package "dupin"} ext revision "$src" --json)
              jq --argjson declared "$declared" \
                '{($declared.package): {revision: $declared.revision, enabled, grant: .grants, deny, bindings}}' <<<"$package"
            done | jq -s --slurpfile bundled "$bundledPath" '
              (map(keys[0]) | group_by(.) | map(select(length > 1) | .[0])) as $twice
              | if $twice != [] then
                  error("programs.dupin.extensions: more than one package is named \($twice | join(", ")) in package.json")
                else {version: 1, packages: ((add // {}) + $bundled[0])} end
            ' > grants.json
            json2toml grants.json "$out"
          '';
      extension = lib.types.submodule (
        { config, ... }:
        {
          options = {
            src = lib.mkOption {
              type = lib.types.path;
              example = lib.literalExpression ''
                pkgs.fetchFromGitHub {
                  owner = "someone";
                  repo = "dupin-word-count";
                  rev = "0123456789abcdef0123456789abcdef01234567";
                  hash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
                }
              '';
              description = ''
                The extension package: a folder with `package.json` and its built JavaScript entry.
                Pin it, for example with `fetchFromGitHub` and a commit `rev` and `hash`.
              '';
            };
            name = lib.mkOption {
              type = lib.types.strMatching "[a-z0-9-]{1,64}";
              default = (lib.importJSON "${config.src}/package.json").name;
              defaultText = lib.literalMD "`name` from the package's `package.json`";
              description = ''
                The name of the package's folder. It defaults to `name` in `package.json`, which
                Nix reads from `src` while evaluating; set it to avoid that. Grants always apply to
                the package by its `package.json` name, read when the configuration builds.
              '';
            };
            grants = lib.mkOption {
              type = lib.types.listOf scope;
              default = [ ];
              example = [
                "http:https://api.example.com"
                "clipboard:write"
              ];
              description = ''
                Capability scopes granted ahead of time, named as the capability broker names
                the scopes the package's `dupin.capabilities` declares, such as
                `http:https://api.example.com`, `files:notes` or `clipboard:write`. They apply to
                exactly this `src`: Dupin's own revision of it is computed when the
                configuration builds. A `files` or `exec` scope also needs its path in
                `bindings`. They are the read-only base; choices made in Dupin can take them
                away but never widen them. macOS still asks for its own permissions on first use.
              '';
            };
            deny = lib.mkOption {
              type = lib.types.listOf scope;
              default = [ ];
              example = [ "clipboard:read" ];
              description = "Scopes the package may never be granted, even from Dupin.";
            };
            bindings = lib.mkOption {
              type = lib.types.attrsOf lib.types.str;
              default = { };
              example = {
                "files:notes" = "/Users/me/Notes";
              };
              description = ''
                The absolute path a granted `files` or `exec` scope is bound to: the file or
                folder it may read, or the program it runs.
              '';
            };
            enable = lib.mkOption {
              type = lib.types.bool;
              default = true;
              description = ''
                Whether the package may use what it is granted. `false` keeps it off for every
                revision, whatever Dupin's own switch says.
              '';
            };
          };
        }
      );
      bundledExtension = lib.types.submodule {
        options = {
          enable = lib.mkOption {
            type = lib.types.nullOr lib.types.bool;
            default = null;
            description = ''
              `true` turns the package on, even one that ships off such as Window Management;
              Dupin's own switch can still turn it off. `false` keeps it off whatever Dupin's
              switch says, so a built-in command it answers says it isn't available. `null`
              leaves it to Dupin.
            '';
          };
          deny = lib.mkOption {
            type = lib.types.listOf scope;
            default = [ ];
            example = [ "calendar" ];
            description = ''
              Scopes the package may never use, although it ships with them granted, named as
              the capability broker names them, such as `calendar`, `clipboard:watch` or
              `windows:arrange`. Each must be one the package declares.
            '';
          };
        };
      };
    in
    {
      options.programs.dupin = {
        enable = lib.mkEnableOption "Dupin, a launcher for Apple silicon Macs";

        package = lib.mkOption {
          type = lib.types.package;
          default = self.packages.${pkgs.stdenv.hostPlatform.system}.dupin;
          defaultText = lib.literalExpression "dupin.packages.\${system}.dupin";
          description = "The Dupin package to install.";
        };

        settings = lib.mkOption {
          inherit (toml) type;
          default = { };
          example = lib.literalExpression ''
            {
              hotkey = "alt+space";
              position = "top";
              input_source = "ascii";
              file_limit = 100;
              aliases = { c = "calendar"; ff = "app:org.mozilla.firefox"; };
              advanced.kinds.file.actions = [
                { title = "Open in Zed"; script = "open-in-zed"; }
              ];
            }
          '';
          description = ''
            Written to {file}`$XDG_CONFIG_HOME/dupin/config.toml`. Leave it empty to keep the
            file out of Nix, so Dupin's Settings window can change it. When set, the keys it
            contains are locked in Settings and other edits are saved, encrypted, in
            Dupin's data directory. `aliases` maps shorthands to commands, apps, web searches and
            files; see <https://github.com/x-ryoppippi/dupin/blob/main/docs/features/keywords.md>.
            `advanced` recomposes the built-in sources, filters and kinds; see
            <https://github.com/x-ryoppippi/dupin/blob/main/docs/features/advanced.md>.
            See <https://github.com/x-ryoppippi/dupin/blob/main/docs/configuration.md>.
          '';
        };

        extensions = lib.mkOption {
          type = lib.types.listOf extension;
          default = [ ];
          description = ''
            Extension packages linked read-only into {file}`$XDG_CONFIG_HOME/dupin/extensions/`,
            with their grants written to {file}`$XDG_CONFIG_HOME/dupin/extension-grants.toml`.
            Extensions still run only once `extensions = true`. See
            <https://github.com/x-ryoppippi/dupin/blob/main/docs/features/extensions.md>.
          '';
        };

        bundledExtensions = lib.mkOption {
          type = lib.types.attrsOf bundledExtension;
          default = { };
          example = lib.literalExpression ''
            {
              window-management.enable = true;
              calendar.deny = [ "calendar" ];
              clipboard-history.enable = false;
            }
          '';
          description = ''
            Switches and denials for the extensions that ship inside Dupin, by their
            `package.json` name, written with the grants of {option}`programs.dupin.extensions`
            to {file}`$XDG_CONFIG_HOME/dupin/extension-grants.toml`. Bundled extensions run
            without `extensions = true`. See
            <https://github.com/x-ryoppippi/dupin/blob/main/docs/features/extensions.md>.
          '';
        };

        launchd.enable = lib.mkEnableOption "starting Dupin at login with a launchd agent";
      };

      config = lib.mkIf cfg.enable {
        assertions = [
          {
            assertion = lib.allUnique (map (extension: extension.name) cfg.extensions);
            message = "programs.dupin.extensions: two packages have the same name.";
          }
          {
            assertion = unknownBundled == [ ];
            message = "programs.dupin.bundledExtensions: programs.dupin.package bundles no ${lib.concatStringsSep ", " unknownBundled}; it bundles ${lib.concatStringsSep ", " bundledNames}.";
          }
          {
            assertion = undeclaredDenials == [ ];
            message = "programs.dupin.bundledExtensions: ${lib.concatStringsSep "; " undeclaredDenials}.";
          }
        ];

        home.packages = [ cfg.package ];

        xdg.configFile = lib.mkMerge [
          {
            "dupin/config.toml" = lib.mkIf (cfg.settings != { }) {
              source = toml.generate "dupin-config.toml" cfg.settings;
            };
            "dupin/extension-grants.toml" = lib.mkIf (cfg.extensions != [ ] || bundledPackages != { }) {
              source = grantsFile;
            };
          }
          (lib.listToAttrs (
            map (
              extension: lib.nameValuePair "dupin/extensions/${extension.name}" { source = extension.src; }
            ) cfg.extensions
          ))
        ];

        launchd.agents.dupin = lib.mkIf cfg.launchd.enable {
          enable = true;
          config = {
            ProgramArguments = [ "${cfg.package}/Applications/Dupin.app/Contents/MacOS/dupin" ];
            RunAtLoad = true;
            # Restart after a crash, not after Quit.
            KeepAlive.SuccessfulExit = false;
            ProcessType = "Interactive";
            # launchd doesn't load the shell's session variables.
            EnvironmentVariables = {
              XDG_CONFIG_HOME = config.xdg.configHome;
            }
            // lib.optionalAttrs config.xdg.enable {
              # Dupin keeps its data, Settings edits included, under XDG_DATA_HOME when the shell exports it.
              XDG_DATA_HOME = config.xdg.dataHome;
            };
          };
        };
      };
    };
}
