{
  pkgs,
  crane,
  native,
  rolldown,
  cargo-hawk,
}:
let
  inherit (native.passthru)
    commonArgs
    packagingArgs
    cargoSrc
    bundleSrc
    ;
  vendored = native.passthru.cargoVendorDir;
  craneLib = (crane.mkLib pkgs).overrideScope (
    _: prev: {
      # Dependencies never see VERSION, so a release bump must not rebuild them. Crate versions
      # stay 0.0.0 (see Cargo.toml). buildPackage calls this too when given no cargoArtifacts.
      buildDepsOnly = args: prev.buildDepsOnly (args // { version = "0.0.0"; });
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
    for source in ${vendored}/*/; do
      index="$out/$(basename "$source")"
      mkdir -p "$index"
      for crate in "$source"*; do
        ln -s "$(readlink -f "$crate")" "$index/"
      done
    done
    substitute ${vendored}/config.toml "$out/config.toml" \
      --replace-fail ${vendored} "$out"
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
  # The checks never read debug info: a failing test still prints where it panicked. Without
  # it, the debug dependency output each lockfile change adds to CI's cache is half the size.
  checkEnv = baseArgs.env // {
    CARGO_PROFILE_DEV_DEBUG = "false";
  };
  debugArgs = baseArgs // {
    env = checkEnv;
    CARGO_PROFILE = "dev";
    cargoExtraArgs = "--offline --locked --workspace";
    nativeBuildInputs = [ pkgs.clippy ];
  };
  latencyArgs = baseArgs // {
    pname = "dupin-latency";
    CARGO_PROFILE = "latency";
    cargoExtraArgs = "--offline --locked --package dupin";
    doCheck = false;
  };
  releaseArtifacts = craneLib.buildDepsOnly releaseArgs;
  debugArtifacts = craneLib.buildDepsOnly debugArgs;
  lintAndTest = craneLib.mkCargoDerivation (
    debugArgs
    // {
      pname = "dupin-lint-and-test";
      cargoArtifacts = debugArtifacts;
      # The extension installer tests fetch from fixture Git repositories, and the extension
      # SDK's tests bundle their fixtures with the Rolldown on PATH.
      nativeBuildInputs = debugArgs.nativeBuildInputs ++ [
        pkgs.git
        rolldown
      ];
      buildPhaseCargoCommand = ''
        cargo clippy --workspace --all-targets --offline --locked -- -D warnings
      '';
      checkPhaseCargoCommand = "cargo test --workspace --offline --locked";
      doCheck = true;
      doInstallCargoArtifacts = false;
      installPhaseCommand = "touch $out";
    }
  );
  # Hawk compiles with its own toolchain, so it can't reuse the other dependency outputs.
  hawkArgs = baseArgs // {
    pname = "dupin-hawk";
    env = checkEnv;
    nativeBuildInputs = [ cargo-hawk ];
    doCheck = false;
  };
  # Exclusions and entry points change what Hawk reports, not what it compiles, so the
  # dependency run leaves out hawk.toml and scripts/hawk.nu, which change with every new crate.
  hawkDepsConfig = pkgs.writeText "hawk.toml" ''
    [[production]]
    package = "dupin"
    bin = "dupin"
    reason = "the dependency build"
  '';
  hawkArtifacts = craneLib.buildDepsOnly (
    hawkArgs
    // {
      buildPhaseCargoCommand = "cargo-hawk check --manifest-path Cargo.toml --config ${hawkDepsConfig}";
    }
  );
  hawk = craneLib.mkCargoDerivation (
    hawkArgs
    // {
      nativeBuildInputs = hawkArgs.nativeBuildInputs ++ [ pkgs.nushell ];
      postPatch = ''
        cp ${../hawk.toml} hawk.toml
      '';
      buildPhaseCargoCommand = "nu ${../scripts/hawk.nu}";
      cargoArtifacts = hawkArtifacts;
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
        hawkArtifacts
        hawk
        cargoVendorDir
        cargoBinary
        latencyBinary
        ;
      inherit (commonArgs) cargoDeps;
      baseline = native;
    };
  }
)
