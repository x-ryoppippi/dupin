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

    # Every dev build here is a check, and the checks never read debug info: a failing test
    # still prints where it panicked. Without it, the debug dependency output each lockfile
    # change adds to CI's cache is half the size. The dependencies need their own entry, as
    # Cargo.toml's override for them outranks the profile's own setting.
    [profile.dev]
    debug = false

    [profile.dev.package."*"]
    debug = false
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
  # The binary scripts/package-app.nu signs: the same build with the telemetry collector's address
  # compiled in, which no other build carries, so only a release can send the usage ping
  # (docs/features/telemetry.md). Only the dupin crate reads it, so the dependencies are shared.
  releaseBinary = craneLib.buildPackage (
    releaseArgs
    // {
      cargoArtifacts = releaseArtifacts;
      dontStrip = true;
      doRemapPathPrefix = false;
      env = releaseArgs.env // {
        DUPIN_TELEMETRY_ENDPOINT = "https://telemetry.dupin.app/v1/ping";
      };
    }
  );
  # Headless-capable, for scripts/latency.nu; never shipped.
  latencyBinary = craneLib.buildPackage (
    latencyArgs
    // {
      cargoArtifacts = craneLib.buildDepsOnly latencyArgs;
      # A binary outside the source tree finds bundled extensions only in an app's Resources,
      # so the ports are measured running, as the released app runs them.
      postInstall = ''
        contents="$out/Dupin.app/Contents"
        mkdir -p "$contents/MacOS" "$contents/Resources"
        mv "$out/bin/dupin" "$contents/MacOS/dupin"
        cp -R extensions "$contents/Resources/extensions"
        ln -s ../Dupin.app/Contents/MacOS/dupin "$out/bin/dupin"
      '';
    }
  );
  # scripts/search-quality.nu's benchmark, on the checks' dependency build; never shipped.
  searchQualityBinary = craneLib.buildPackage (
    debugArgs
    // {
      pname = "dupin-search-quality";
      cargoArtifacts = debugArtifacts;
      cargoExtraArgs = "--offline --locked --package dupin-search-quality";
      doCheck = false;
    }
  );
  # Dupin.app around a release binary.
  app =
    binary: passthru:
    pkgs.stdenv.mkDerivation (
      packagingArgs
      // {
        inherit (commonArgs) pname version;
        inherit passthru;
        src = bundleSrc;
        dontConfigure = true;
        dontBuild = true;
        installPhase = ''
          runHook preInstall
          mkdir -p "$out/bin"
          install -m755 ${binary}/bin/dupin "$out/bin/dupin"
          runHook postInstall
        '';
      }
    );
in
app cargoBinary {
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
    searchQualityBinary
    ;
  inherit (commonArgs) cargoDeps;
  inherit (native) bundledExtensions thirdPartyNotices;
  baseline = native;
  release = app releaseBinary { };
}
