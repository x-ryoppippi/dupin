{
  perSystem =
    { config, pkgs, ... }:
    {
      devShells.default = pkgs.mkShell {
        inputsFrom = [ config.packages.dupin.baseline ];
        packages =
          with pkgs;
          [
            clippy
            rustfmt
            rust-analyzer
            nushell
            nodejs
            # `dupin ext build` falls back to the Rolldown on PATH for a package without its own.
            config.packages.rolldown
            # Type-checks extensions/ as `checks.extensions-typecheck` does.
            typescript
            config.packages.pnpm
            config.packages.blacksmith
            just
            config.packages.cargo-hawk
            nufmt
            config.treefmt.build.wrapper
            typos
            oxfmt
            actionlint
            zizmor
            gitleaks
            # Compresses the documentation images scripts/docs-images.nu renders.
            oxipng
            # Caption and encode the documentation clips scripts/docs-images.nu renders.
            imagemagick
            libwebp
          ]
          ++ config.pre-commit.settings.enabledPackages;
        # The captions' typeface, the same on every Mac.
        DUPIN_CAPTION_FONT = "${pkgs.inter}/share/fonts/truetype/InterVariable.ttf";
        # Installs the pre-commit and pre-push hooks from nix/git-hooks.nix.
        shellHook = config.pre-commit.shellHook;
      };
    };
}
