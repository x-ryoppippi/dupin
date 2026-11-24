# The Cargo dependencies, vendored from Cargo.lock: crates.io packages by their lock checksums,
# so a lock change needs no hash, and Git packages by the hashes below, one per locked rev.
{ craneLib }:
let
  # Without a hash crane clones a Git source with builtins.fetchGit while evaluating, so every
  # evaluation would clone it, even when the binary cache has every output.
  outputHashes = {
    "git+https://github.com/anomalyco/rune?rev=fcef33a2f60db30a7a872844464f84ff26f3fc75#fcef33a2f60db30a7a872844464f84ff26f3fc75" =
      "sha256-Ew4j+TTT6EJQHwcXx2fffq/xcrnZAlI7yD7zWBQDkG4=";
  };
  lockedGitSources = builtins.filter (source: builtins.substring 0 4 source == "git+") (
    map (package: package.source or "") (builtins.fromTOML (builtins.readFile ../Cargo.lock)).package
  );
  unhashed = builtins.filter (source: !(outputHashes ? ${source})) lockedGitSources;
in
assert
  unhashed == [ ]
  || throw ''
    nix/cargo-vendor.nix has no hash for ${builtins.head unhashed}.
    For Rune, run `nu scripts/update-rune.nu`; for another Git source, add it to outputHashes
    with the hash "" and build .#dupin, and Nix prints the hash to use.
  '';
craneLib.vendorCargoDeps {
  cargoLock = ../Cargo.lock;
  inherit outputHashes;
  # Rune's repository vendors crates copied from crates.io, Cargo.toml.orig included, and
  # `cargo package --list`, which crane lists a Git crate's files with, rejects that name.
  overrideVendorGitCheckout =
    _: drv:
    drv.overrideAttrs (old: {
      preInstall = (old.preInstall or "") + ''
        find . -name Cargo.toml.orig -delete
      '';
    });
}
