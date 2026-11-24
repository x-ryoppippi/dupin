{ inputs, ... }:
{
  perSystem =
    { lib, pkgs, ... }:
    {
      # cargo-hawk.nix fetches the Apple silicon release.
      packages = lib.optionalAttrs pkgs.stdenv.hostPlatform.isDarwin {
        cargo-hawk = (pkgs.extend inputs.rust-overlay.overlays.default).callPackage ./cargo-hawk.nix { };
      };
    };
}
