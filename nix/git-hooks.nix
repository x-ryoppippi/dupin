{ lib, ... }:
{
  perSystem =
    { config, pkgs, ... }:
    {
      pre-commit = {
        # The same checks run in `nix flake check` (checks.treefmt, checks.gitleaks).
        check.enable = false;
        settings = {
          package = pkgs.prek;
          hooks = {
            treefmt = {
              enable = true;
              name = "treefmt";
              entry = lib.getExe config.treefmt.build.wrapper;
              files = ".*";
              pass_filenames = true;
              require_serial = true;
              stages = [ "pre-commit" ];
              priority = 10;
            };
            gitleaks-protect = {
              enable = true;
              name = "gitleaks";
              entry = "${lib.getExe pkgs.gitleaks} protect --staged --config .gitleaks.toml";
              pass_filenames = false;
              always_run = true;
              stages = [ "pre-commit" ];
              priority = 20;
            };
            treefmt-check = {
              enable = true;
              name = "treefmt";
              entry = "${lib.getExe config.treefmt.build.wrapper} --fail-on-change";
              files = ".*";
              pass_filenames = false;
              stages = [ "pre-push" ];
            };
            gitleaks-detect = {
              enable = true;
              name = "gitleaks";
              entry = "${lib.getExe pkgs.gitleaks} detect --config .gitleaks.toml";
              pass_filenames = false;
              always_run = true;
              stages = [ "pre-push" ];
            };
          };
        };
      };
    };
}
