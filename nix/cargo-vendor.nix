# The Cargo dependencies, vendored from Cargo.lock alone: crates.io packages by their lock
# checksums and Git packages by their locked rev, so no hash has to follow the lock.
{ craneLib }:
craneLib.vendorCargoDeps {
  cargoLock = ../Cargo.lock;
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
