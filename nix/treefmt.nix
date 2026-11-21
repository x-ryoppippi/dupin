{ lib, ... }:
{
  perSystem =
    { pkgs, ... }:
    {
      treefmt = {
        projectRootFile = "flake.nix";

        programs = {
          deadnix.enable = true;
          nixfmt.enable = true;
          statix.enable = true;
          rustfmt = {
            enable = true;
            edition = "2024";
          };
          typos = {
            enable = true;
            configFile = "./typos.toml";
          };
        };

        settings.global.excludes = [
          # The logo is made and maintained elsewhere.
          "assets/**"
          ".github/tagpr-template.md"
          "Cargo.lock"
          "flake.lock"
          "VERSION"
          "*.png"
        ];

        settings.formatter = {
          deadnix.priority = 1;
          statix.priority = 2;
          nixfmt.priority = 3;
          oxfmt = {
            command = lib.getExe pkgs.oxfmt;
            options = [ "--no-error-on-unmatched-pattern" ];
            includes = [ "*" ];
            priority = 4;
          };
          actionlint = {
            command = lib.getExe pkgs.actionlint;
            includes = [
              ".github/workflows/*.yaml"
              ".github/workflows/*.yml"
            ];
            priority = 5;
          };
          zizmor = {
            command = lib.getExe pkgs.zizmor;
            options = [
              "--offline"
              "--min-severity"
              "high"
              "--min-confidence"
              "high"
            ];
            includes = [
              ".github/workflows/*.yaml"
              ".github/workflows/*.yml"
            ];
            priority = 6;
          };
          nufmt = {
            command = lib.getExe pkgs.nufmt;
            includes = [ "*.nu" ];
            priority = 7;
          };
          rustfmt.priority = 8;
        };
      };
    };
}
