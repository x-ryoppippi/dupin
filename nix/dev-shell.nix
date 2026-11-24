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
            # `dupin ext build` bundles TypeScript extensions with Bun or esbuild from PATH.
            esbuild
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
          ]
          ++ config.pre-commit.settings.enabledPackages;
        # Installs the pre-commit and pre-push hooks from nix/git-hooks.nix.
        shellHook = config.pre-commit.shellHook;
      };
    };
}
