{
  lib,
  rustPlatform,
  resvg,
  python3Packages,
}:
let
  root = ./.;
  # tagpr bumps VERSION; crate versions stay 0.0.0 (see Cargo.toml).
  version = lib.fileContents (root + /VERSION);

  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      (root + /Cargo.toml)
      (root + /Cargo.lock)
      (root + /rustfmt.toml)
      (root + /crates)
      # The backdrop shader examples, which a test compiles.
      (root + /examples/shaders)
      (root + /packaging)
      (root + /assets/logo/dupin.svg)
    ];
  };

  # fetchCargoVendor, not importCargoLock: Rune vendors a patched tiny-skia-path as a path
  # dependency while resvg keeps the published one, so the lock holds two tiny-skia-path
  # 0.11.4 packages, and importCargoLock's name-version directories collide.
  # Update the hash whenever Cargo.lock changes: `nu scripts/update-cargo-hash.nu`.
  # The name carries Cargo.lock's hash. A fixed-output derivation whose hash didn't change is
  # taken from the store as it is, so with a constant name a stale vendor tree from before a
  # lock change passed every local build and failed only on a clean CI store.
  cargoDeps = rustPlatform.fetchCargoVendor {
    inherit src;
    name = "dupin-vendor-${builtins.substring 0 16 (builtins.hashFile "sha256" (root + /Cargo.lock))}";
    hash = "sha256-C+cO2r+EL9LfmdaEm8df4807K8TR1G6usVarlTNjwBA=";
  };

  # What every Rust derivation of the workspace shares (the package here, clippy and the
  # tests in nix/checks.nix). The toolchain is nixpkgs' own, which matches the version
  # Rune's rust-toolchain.toml pins.
  commonArgs = {
    pname = "dupin";
    inherit version src cargoDeps;
    # Rune workaround: rune-text embeds its default fonts with
    # include_bytes!("../../../rune/assets/..."), a path into the sibling rune crate that
    # only exists in a whole-repository checkout. Vendoring copies each crate on its own,
    # so give the vendored rune crate the directory name that path expects.
    postPatch = ''
      ln -s rune-0.0.1 "$NIX_BUILD_TOP/$(stripHash "$cargoDeps")/source-git-0/rune"
    '';
  };
in
rustPlatform.buildRustPackage (
  commonArgs
  // {
    cargoBuildFlags = [
      "--package"
      "dupin"
    ];
    # Tests run in the lint-and-test check, in debug as cargo runs them.
    doCheck = false;
    # The app icon is drawn from the logo here, so no generated image is committed. iconutil
    # ships only with macOS, outside the sandbox; icnsutil writes the same icns.
    nativeBuildInputs = [
      resvg
      python3Packages.icnsutil
    ];
    postInstall = ''
      app=$out/Applications/Dupin.app/Contents
      mkdir -p $app/MacOS
      mv $out/bin/dupin $app/MacOS/dupin
      substitute packaging/Info.plist $app/Info.plist --subst-var-by version ${version}

      # Every size an iconset has: 16 to 512 points, each also at 2x (up to 1024 pixels).
      iconset=$(mktemp -d)
      for size in 16 32 128 256 512; do
        resvg -w $size -h $size assets/logo/dupin.svg $iconset/icon_''${size}x''${size}.png
        resvg -w $((size * 2)) -h $((size * 2)) assets/logo/dupin.svg $iconset/icon_''${size}x''${size}@2x.png
      done
      mkdir -p $app/Resources
      icnsutil compose --toc $app/Resources/dupin.icns $iconset/*.png
      icnsutil test $app/Resources/dupin.icns
      ln -s $app/MacOS/dupin $out/bin/dupin
    '';
    passthru = { inherit commonArgs; };
    meta = {
      description = "A lightweight resident launcher for Apple silicon Macs";
      homepage = "https://dupin.app";
      license = lib.licenses.gpl3Plus;
      mainProgram = "dupin";
      platforms = [ "aarch64-darwin" ];
    };
  }
)
