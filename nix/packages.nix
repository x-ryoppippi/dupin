{
  perSystem =
    { pkgs, ... }:
    let
      dupin = pkgs.callPackage ../package.nix { };
    in
    {
      packages = {
        inherit dupin;
        default = dupin;
      };
    };
}
