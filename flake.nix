{
  description = "Dupin: a lightweight launcher for Apple silicon Macs";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    # agent-desktop for the E2E job; its own nixpkgs keeps cache.numtide.com hits.
    llm-agents.url = "github:numtide/llm-agents.nix";
  };

  outputs =
    { self, nixpkgs, ... }:
    let
      # Apple silicon macOS is the only target.
      system = "aarch64-darwin";
      pkgs = nixpkgs.legacyPackages.${system};
      inherit (pkgs) lib;
      # tagpr bumps VERSION; crate versions stay 0.0.0 (see Cargo.toml).
      version = lib.fileContents ./VERSION;

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./rustfmt.toml
          ./crates
          ./packaging
        ];
      };

      # fetchCargoVendor, not importCargoLock: Rune vendors a patched tiny-skia-path as a
      # path dependency while resvg keeps the published one, so the lock holds two
      # tiny-skia-path 0.11.4 packages, and importCargoLock's name-version directories collide.
      # Update the hash whenever Cargo.lock changes. Every Rust derivation shares this one.
      cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
        inherit src;
        name = "dupin-vendor";
        hash = "sha256-U4CblbqunzzNq3ULcWINogmkQYVl8nWuDFq47sVupWQ=";
      };

      # The Rust toolchain from nixpkgs matches the version Rune's rust-toolchain.toml pins.
      rustPackage =
        args:
        pkgs.rustPlatform.buildRustPackage (
          {
            pname = "dupin";
            inherit version src cargoDeps;
            # Rune workaround: rune-text embeds its default fonts with
            # include_bytes!("../../../rune/assets/..."), a path into the sibling rune crate
            # that only exists in a whole-repository checkout. Vendoring copies each crate on
            # its own, so give the vendored rune crate the directory name that path expects.
            postPatch = ''
              ln -s rune-0.0.1 "$NIX_BUILD_TOP/$(stripHash "$cargoDeps")/source-git-0/rune"
            '';
          }
          // args
        );

      dupin = rustPackage {
        cargoBuildFlags = [ "--package" "dupin" ];
        # Tests run in the `test` check, in debug as cargo runs them.
        doCheck = false;
        postInstall = ''
          app=$out/Applications/Dupin.app/Contents
          mkdir -p $app/MacOS
          mv $out/bin/dupin $app/MacOS/dupin
          substitute packaging/Info.plist $app/Info.plist --subst-var-by version ${version}
          ln -s $app/MacOS/dupin $out/bin/dupin
        '';
        meta = {
          description = "A lightweight resident launcher for Apple silicon Macs";
          homepage = "https://dupin.app";
          mainProgram = "dupin";
          platforms = [ system ];
        };
      };

      # Clippy and the tests share one debug build of the dependencies.
      lint-and-test = rustPackage {
        pname = "dupin-lint-and-test";
        nativeBuildInputs = [ pkgs.clippy ];
        buildPhase = ''
          runHook preBuild
          cargo clippy --workspace --all-targets --offline -- -D warnings
          runHook postBuild
        '';
        checkPhase = ''
          runHook preCheck
          cargo test --workspace --offline
          runHook postCheck
        '';
        installPhase = "touch $out";
      };
    in
    {
      packages.${system} = {
        inherit dupin;
        default = dupin;
      };

      checks.${system} = {
        inherit dupin lint-and-test;
        fmt = pkgs.runCommand "dupin-fmt" { nativeBuildInputs = [ pkgs.cargo pkgs.rustfmt ]; } ''
          cd ${src}
          cargo fmt --all --check
          touch $out
        '';
      };

      devShells.${system}.default = pkgs.mkShell {
        inputsFrom = [ dupin ];
        packages = with pkgs; [
          clippy
          rustfmt
          rust-analyzer
          jq
        ];
      };

      formatter.${system} = pkgs.nixfmt;
    };
}
