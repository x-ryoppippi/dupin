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
                The package's name, which names its folder. It defaults to `name` in
                `package.json`, which Nix reads from `src` while evaluating; set it to avoid that.
              '';
            };
            grants = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              default = [ ];
              example = [
                "http:https://api.example.com"
                "clipboard"
              ];
              description = ''
                Capability scopes granted ahead of time, as `capability` or `capability:scope`
                from the package's `dupin.capabilities`. They are the read-only base; grants
                chosen in Dupin overlay them. macOS still asks for its own permissions on first use.
              '';
            };
          };
        }
      );
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
            }
          '';
          description = ''
            Written to {file}`$XDG_CONFIG_HOME/dupin/config.toml`. Leave it empty to keep the
            file out of Nix, so Dupin's Settings window can change it. When set, the keys it
            contains are locked in Settings and other edits are saved to `settings.toml` in
            Dupin's data directory. See
            <https://github.com/x-ryoppippi/dupin/blob/main/docs/configuration.md>.
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

        launchd.enable = lib.mkEnableOption "starting Dupin at login with a launchd agent";
      };

      config = lib.mkIf cfg.enable {
        assertions = [
          {
            assertion = lib.allUnique (map (extension: extension.name) cfg.extensions);
            message = "programs.dupin.extensions: two packages have the same name.";
          }
        ];

        home.packages = [ cfg.package ];

        xdg.configFile = lib.mkMerge [
          {
            "dupin/config.toml" = lib.mkIf (cfg.settings != { }) {
              source = toml.generate "dupin-config.toml" cfg.settings;
            };
            "dupin/extension-grants.toml" = lib.mkIf (cfg.extensions != [ ]) {
              source = toml.generate "dupin-extension-grants.toml" (
                lib.listToAttrs (
                  map (extension: lib.nameValuePair extension.name { inherit (extension) grants; }) cfg.extensions
                )
              );
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
              # The settings.toml overlay lives under XDG_DATA_HOME when the shell exports it.
              XDG_DATA_HOME = config.xdg.dataHome;
            };
          };
        };
      };
    };
}
