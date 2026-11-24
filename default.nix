{
  pkgs ? import <nixpkgs> { },
}:
let
  crane = (builtins.fromJSON (builtins.readFile ./flake.lock)).nodes.crane.locked;
  craneLib = import (builtins.fetchTarball {
    url = "https://github.com/${crane.owner}/${crane.repo}/archive/${crane.rev}.tar.gz";
    sha256 = crane.narHash;
  }) { inherit pkgs; };
in
pkgs.callPackage ./package.nix {
  cargoVendorDir = import ./nix/cargo-vendor.nix { inherit craneLib; };
}
