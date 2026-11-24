{
  lib,
  runCommand,
  rustPlatform,
  cargoVendorDir,
  resvg,
  oxipng,
  python3Packages,
  inter,
}:
let
  root = ./.;
  # tagpr bumps VERSION; crate versions stay 0.0.0 (see Cargo.toml).
  version = lib.fileContents (root + /VERSION);

  cargoFiles = lib.fileset.unions [
    (root + /Cargo.toml)
    (root + /Cargo.lock)
    (root + /VERSION)
    (root + /rustfmt.toml)
    (root + /crates)
    # The extension declarations `dupin ext new` copies into a package.
    (root + /sdk)
    # The backdrop shader examples, which a test compiles.
    (root + /examples/shaders)
    # The JavaScript word counter, which tests load and run.
    (root + /examples/extensions/word-count-js)
    # The bundled extensions, which a test build finds in the source tree.
    (root + /extensions)
  ];
  bundleFiles = lib.fileset.unions [
    (root + /packaging)
    (root + /assets/logo/dupin.svg)
    (root + /extensions)
  ];
  # Each bundled package by the name the broker knows it by after `bundled:`, `name` in its
  # package.json, with the scope keys its capabilities declare. checks.bundled-extension-scopes
  # holds these scope keys to what Dupin itself reads.
  scopesOf =
    capabilities:
    lib.concatLists (
      lib.mapAttrsToList (
        kind: declared:
        if
          lib.elem kind [
            "storage"
            "dictionary"
            "calendar"
          ]
        then
          [ kind ]
        else if
          lib.elem kind [
            "clipboard"
            "windows"
          ]
        then
          map (access: "${kind}:${access}") declared
        else if kind == "http" then
          map (entry: "http:${entry.origin}") declared
        else if kind == "files" then
          map (name: "files:${name}") (lib.attrNames declared)
        else if kind == "exec" then
          map (
            name:
            if lib.hasPrefix "@script:" name then
              "script:${lib.removePrefix "@script:" name}"
            else
              "exec:${name}"
          ) (lib.attrNames declared)
        else
          throw "package.nix doesn't know the scope keys of the `${kind}` capability"
      ) capabilities
    );
  bundledExtensions = lib.listToAttrs (
    lib.mapAttrsToList (
      folder: _:
      let
        manifest = lib.importJSON (root + "/extensions/${folder}/package.json");
      in
      lib.nameValuePair manifest.name (scopesOf (manifest.dupin.capabilities or { }))
    ) (lib.filterAttrs (_: type: type == "directory") (builtins.readDir (root + /extensions)))
  );
  cargoSrc = lib.fileset.toSource {
    inherit root;
    fileset = cargoFiles;
  };
  bundleSrc = lib.fileset.toSource {
    inherit root;
    fileset = bundleFiles;
  };
  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      cargoFiles
      bundleFiles
    ];
  };

  # cargoSetupHook's layout over nix/cargo-vendor.nix: config.toml with `@vendor@` for the
  # copy it makes, and the Cargo.lock it checks the source's against.
  cargoDeps = runCommand "dupin-vendor" { } ''
    mkdir -p "$out/.cargo"
    for source in ${cargoVendorDir}/*/; do
      ln -s "$(readlink -f "$source")" "$out/$(basename "$source")"
    done
    substitute ${cargoVendorDir}/config.toml "$out/.cargo/config.toml" \
      --replace-fail ${cargoVendorDir} @vendor@
    cp ${root + /Cargo.lock} "$out/Cargo.lock"
  '';

  # What every Rust derivation of the workspace shares (the package here, clippy and the
  # tests in nix/checks.nix). The toolchain is nixpkgs' own, which matches the version
  # Rune's rust-toolchain.toml pins.
  commonArgs = {
    pname = "dupin";
    inherit version src cargoDeps;
    # RUSTFLAGS would replace cargoSetupHook's target-specific flags.
    prePatch = ''
      export NIX_RUSTFLAGS="''${NIX_RUSTFLAGS-} --remap-path-prefix=$cargoDepsCopy=/vendor"
    '';
  };
  packagingArgs = {
    # The app icon is drawn from the logo here, so no generated image is committed. iconutil
    # ships only with macOS, outside the sandbox; icnsutil writes the same icns.
    nativeBuildInputs = [
      resvg
      oxipng
      python3Packages.icnsutil
    ];
    # cargoBuildHook overrides the profile's `strip` and leaves stripping to stdenv, whose
    # default flags include `-p`, which llvm-strip rejects for Mach-O; the hook ignores the
    # failure and leaves every symbol in place.
    stripAllList = [ "Applications/Dupin.app/Contents/MacOS" ];
    stripAllFlags = [ "--strip-all" ];
    postInstall = ''
      app=$out/Applications/Dupin.app/Contents
      mkdir -p $app/MacOS
      mv $out/bin/dupin $app/MacOS/dupin
      substitute packaging/Info.plist $app/Info.plist --subst-var-by version ${version}

      # Every size an iconset has: 16 to 512 points, each also at 2x (up to 1024 pixels). The
      # nightly icon's ribbon is text, and the sandbox has no system fonts.
      icns() {
        local iconset
        iconset=$(mktemp -d)
        for size in 16 32 128 256 512; do
          resvg --skip-system-fonts --use-fonts-dir ${inter}/share/fonts -w $size -h $size "$1" $iconset/icon_''${size}x''${size}.png
          resvg --skip-system-fonts --use-fonts-dir ${inter}/share/fonts -w $((size * 2)) -h $((size * 2)) "$1" $iconset/icon_''${size}x''${size}@2x.png
        done
        oxipng -o 4 --nx "$iconset"/*.png
        icnsutil compose --toc "$2" $iconset/*.png
        icnsutil test "$2"
      }
      mkdir -p $app/Resources
      icns assets/logo/dupin.svg $app/Resources/dupin.icns
      # scripts/package-app.nu points a nightly build at this one.
      icns packaging/nightly-icon.svg $app/Resources/dupin-nightly.icns
      # Each package folder, without extensions/tsconfig.json, which only the type check reads.
      mkdir $app/Resources/extensions
      for package in extensions/*/; do
        cp -R "''${package%/}" $app/Resources/extensions/
      done
      ln -s $app/MacOS/dupin $out/bin/dupin
    '';
    meta = {
      description = "A lightweight resident launcher for Apple silicon Macs";
      homepage = "https://dupin.app";
      license = lib.licenses.gpl3Plus;
      mainProgram = "dupin";
      platforms = [ "aarch64-darwin" ];
    };
  };
in
rustPlatform.buildRustPackage (
  commonArgs
  // packagingArgs
  // {
    cargoBuildFlags = [
      "--package"
      "dupin"
    ];
    # Tests run in the lint-and-test check, in debug as cargo runs them.
    doCheck = false;
    passthru = {
      inherit
        commonArgs
        packagingArgs
        cargoVendorDir
        cargoSrc
        bundleSrc
        bundledExtensions
        ;
    };
  }
)
