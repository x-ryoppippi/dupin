{
  pkgs,
  crane,
  native,
  rolldown,
}:
let
  inherit (native.passthru)
    commonArgs
    packagingArgs
    cargoSrc
    bundleSrc
    ;
  craneLib = (crane.mkLib pkgs).overrideScope (
    _: _: {
      cargo = pkgs.buildPackages.cargo-auditable-cargo-wrapper.override {
        cargo = pkgs.rustPlatform.rust.cargo;
        inherit (pkgs) cargo-auditable;
      };
      rustc = pkgs.rustPlatform.rust.rustc;
      inherit (pkgs) clippy;
      inherit (pkgs) rustfmt;
    }
  );
  # Match cargoSetupHook's target defaults so cached and baseline code generation agree.
  cargoVendorDir = pkgs.runCommand "dupin-cargo-vendor-config" { } ''
    mkdir -p "$out"
    # Per-crate links let Crane remove references to the original source store path.
    for source in ${commonArgs.cargoDeps}/source-*; do
      index="$out/$(basename "$source")"
      mkdir -p "$index"
      for crate in "$source"/*; do
        ln -s "$crate" "$index/"
      done
    done
    substitute ${commonArgs.cargoDeps}/.cargo/config.toml "$out/config.toml" \
      --replace-fail '@vendor@' "$out"
    cat >> "$out/config.toml" <<'EOF'

    [build]
    target = "${pkgs.stdenv.hostPlatform.rust.rustcTarget}"

    [target."${pkgs.stdenv.hostPlatform.rust.rustcTarget}"]
    linker = "${pkgs.stdenv.cc}/bin/cc"
    rustflags = [ "-Cforce-frame-pointers=yes" ]
    EOF
  '';
  baseArgs = builtins.removeAttrs commonArgs [ "cargoDeps" ] // {
    src = cargoSrc;
    inherit cargoVendorDir;
    # Crane compiles through the adapter rather than cargoSetupHook's copied tree.
    prePatch = ''
      export NIX_RUSTFLAGS="''${NIX_RUSTFLAGS-} --remap-path-prefix=${cargoVendorDir}=/vendor"
    '';
    env.CARGO_PROFILE_RELEASE_STRIP = "false";
  };
  releaseArgs = baseArgs // {
    CARGO_PROFILE = "release";
    cargoExtraArgs = "--offline --locked --package dupin";
    doCheck = false;
  };
  debugArgs = baseArgs // {
    CARGO_PROFILE = "dev";
    cargoExtraArgs = "--offline --locked --workspace";
    nativeBuildInputs = [ pkgs.clippy ];
  };
  latencyArgs = baseArgs // {
    pname = "dupin-latency";
    CARGO_PROFILE = "latency";
    cargoExtraArgs = "--offline --locked --package dupin --features tooling";
    doCheck = false;
  };
  releaseArtifacts = craneLib.buildDepsOnly releaseArgs;
  debugArtifacts = craneLib.buildDepsOnly debugArgs;
  lintAndTest = craneLib.mkCargoDerivation (
    debugArgs
    // {
      pname = "dupin-lint-and-test";
      cargoArtifacts = debugArtifacts;
      # The extension SDK's tests bundle their fixtures with the Rolldown on PATH.
      nativeBuildInputs = debugArgs.nativeBuildInputs ++ [ rolldown ];
      buildPhaseCargoCommand = ''
        cargo clippy --workspace --all-targets --offline --locked -- -D warnings
      '';
      checkPhaseCargoCommand = "cargo test --workspace --offline --locked";
      doCheck = true;
      doInstallCargoArtifacts = false;
      installPhaseCommand = "touch $out";
    }
  );
  cargoBinary = craneLib.buildPackage (
    releaseArgs
    // {
      cargoArtifacts = releaseArtifacts;
      dontStrip = true;
      doRemapPathPrefix = false;
    }
  );
  # Headless-capable, for scripts/latency.nu; never shipped.
  latencyBinary = craneLib.buildPackage (
    latencyArgs
    // {
      cargoArtifacts = craneLib.buildDepsOnly latencyArgs;
    }
  );
in
pkgs.stdenv.mkDerivation (
  packagingArgs
  // {
    inherit (commonArgs) pname version;
    src = bundleSrc;
    dontConfigure = true;
    dontBuild = true;
    installPhase = ''
      runHook preInstall
      mkdir -p "$out/bin"
      install -m755 ${cargoBinary}/bin/dupin "$out/bin/dupin"
      runHook postInstall
    '';
    passthru = {
      inherit
        commonArgs
        packagingArgs
        releaseArtifacts
        debugArtifacts
        lintAndTest
        cargoVendorDir
        cargoBinary
        latencyBinary
        ;
      inherit (commonArgs) cargoDeps;
      baseline = native;
    };
  }
)
