{ inputs, ... }:
{
  perSystem =
    { pkgs, ... }:
    {
      packages.cargo-hawk =
        (pkgs.extend inputs.rust-overlay.overlays.default).callPackage ./cargo-hawk.nix
          { };
    };
}
