{
  perSystem =
    { pkgs, ... }:
    let
      dupin = pkgs.callPackage ../package.nix { };
      # scripts/package-app.nu lays out Dupin.dmg over this.
      dmg-background =
        pkgs.runCommand "dupin-dmg-background"
          {
            nativeBuildInputs = [ pkgs.resvg ];
          }
          ''
            mkdir $out
            for scale in 1 2; do
              name=background; [ $scale = 2 ] && name=background@2x
              resvg --skip-system-fonts --use-fonts-dir ${pkgs.inter}/share/fonts \
                -w $((660 * scale)) -h $((400 * scale)) ${../packaging/dmg/background.svg} $out/$name.png
            done
          '';
    in
    {
      packages = {
        inherit dupin dmg-background;
        default = dupin;
      };
    };
}
