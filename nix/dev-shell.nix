{
  perSystem =
    {
      config,
      lib,
      pkgs,
      ...
    }:
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
            just
            nufmt
            config.treefmt.build.wrapper
            typos
            oxfmt
            actionlint
            zizmor
            gitleaks
          ]
          ++ lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
            config.packages.blacksmith
            config.packages.cargo-hawk
          ]
          ++ config.pre-commit.settings.enabledPackages;
        # `cargo run -p dupin` finds the libraries Dupin opens at run time through the RUNPATH.
        env = lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
          CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS = "-C link-arg=-Wl,-rpath,${lib.makeLibraryPath config.packages.dupin.baseline.runtimeLibraries}";
        };
        # Installs the pre-commit and pre-push hooks from nix/git-hooks.nix.
        shellHook = config.pre-commit.shellHook;
      };
    };
}
