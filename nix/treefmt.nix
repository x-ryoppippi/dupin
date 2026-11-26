{ inputs, ... }:
let
  treefmt =
    { lib, pkgs, ... }:
    {
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
          # Report only: autofix.ci would commit whatever typos rewrote, UK spellings turned
          # American by the en-us locale among them, and a typo's fix needs a person.
          autoFix = false;
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
        # Generated from crates/dupin-telemetry, whose test compares it byte for byte.
        "telemetry/src/v1/ping-schema.gen.ts"
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
in
{
  perSystem.treefmt = treefmt;

  # The flake targets only aarch64-darwin, but the autofix.ci workflow formats on a Linux
  # runner, which costs far less than macOS.
  flake.formatter.x86_64-linux = inputs.treefmt-nix.lib.mkWrapper inputs.nixpkgs.legacyPackages.x86_64-linux treefmt;
}
