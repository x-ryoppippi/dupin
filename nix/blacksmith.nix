{
  lib,
  stdenvNoCC,
  fetchurl,
  makeWrapper,
  openssh,
  rsync,
}:
let
  version = "0.5.3";
in
stdenvNoCC.mkDerivation {
  pname = "blacksmith";
  inherit version;

  src = fetchurl {
    url = "https://clireleases.blacksmith.sh/cli/v${version}/darwin/arm64/blacksmith";
    sha256 = "51a13e1e2f44aa79c9bbce5bbee849ef749aea95cb1201ef3fea3108520133f8";
  };

  dontUnpack = true;
  nativeBuildInputs = [ makeWrapper ];

  installPhase = ''
    runHook preInstall
    install -Dm755 $src $out/libexec/blacksmith
    makeWrapper $out/libexec/blacksmith $out/bin/blacksmith \
      --set BLACKSMITH_DISABLE_AUTO_UPDATE 1 \
      --prefix PATH : ${
        lib.makeBinPath [
          openssh
          rsync
        ]
      }
    runHook postInstall
  '';

  meta = {
    description = "Blacksmith CLI for testboxes";
    homepage = "https://docs.blacksmith.sh";
    mainProgram = "blacksmith";
    platforms = [ "aarch64-darwin" ];
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
  };
}
