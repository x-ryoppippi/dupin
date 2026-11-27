{ lib, ... }:
{
  perSystem =
    { config, pkgs, ... }:
    let
      settings = config.pre-commit.settings;
      git = lib.getExe settings.gitPackage;
      prek = lib.getExe settings.package;
      hookTypes = lib.concatMapStringsSep " " (stage: "-t ${stage}") (
        lib.remove "manual" settings.installStages
      );
    in
    {
      options.gitHooks.shellHook = lib.mkOption {
        type = lib.types.str;
        readOnly = true;
        description = "Installs the hooks below for every worktree of the clone the shell starts in.";
      };

      config = {
        # Replaces git-hooks.nix's installer, which links the config into one worktree's
        # toplevel and sets a relative core.hooksPath, so other worktrees get no config or
        # no hooks. Hooks and config both belong in the common Git directory, with absolute paths.
        gitHooks.shellHook = ''
          if common_dir=$(${git} rev-parse --path-format=absolute --git-common-dir 2>/dev/null); then
            hooks_config="$common_dir/pre-commit-config.yaml"
            if [[ $(readlink "$hooks_config") != ${settings.configFile} ]]; then
              nix-store --add-root "$hooks_config" --indirect --realise ${settings.configFile} >/dev/null
            fi
            ${git} config --local core.hooksPath "$common_dir/hooks"
            ${prek} install --quiet --force --git-dir "$common_dir" --config "$hooks_config" ${hookTypes}
          fi
          unset common_dir hooks_config
          export PATH=${settings.package}/bin:$PATH
        '';

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
    };
}
