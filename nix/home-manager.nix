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

        launchd.enable = lib.mkEnableOption "starting Dupin at login with a launchd agent";
      };

      config = lib.mkIf cfg.enable {
        home.packages = [ cfg.package ];

        xdg.configFile."dupin/config.toml" = lib.mkIf (cfg.settings != { }) {
          source = toml.generate "dupin-config.toml" cfg.settings;
        };

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
