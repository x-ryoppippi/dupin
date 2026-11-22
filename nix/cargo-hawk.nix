{
  fetchurl,
  lib,
  stdenv,
  stdenvNoCC,
  makeWrapper,
  rust-bin,
}:
let
  version = "0.1.18";
  rust = rust-bin.stable."1.100.0".minimal;
in
stdenvNoCC.mkDerivation {
  pname = "cargo-hawk";
  inherit version;
  src = fetchurl {
    url = "https://github.com/astral-sh/hawk/releases/download/${version}/cargo-hawk-aarch64-apple-darwin.tar.gz";
    hash = "sha256-lrqfFQOhlOuHE/AG3yhugqCSmfP9c+su/5jMPODUzYI=";
  };
  sourceRoot = "cargo-hawk-aarch64-apple-darwin";
  nativeBuildInputs = [ makeWrapper ];
  installPhase = ''
    runHook preInstall
    mkdir -p "$out/bin" "$out/libexec/hawk-tools"
    install -m555 cargo-hawk cargo-hawk-driver cargo-hawk-rustdoc "$out/bin/"
    # Hawk's Rust LLVM must not override Clang's when it spawns the linker.
    for tool in cc c++ clang clang++; do
      makeWrapper "${stdenv.cc}/bin/$tool" "$out/libexec/hawk-tools/$tool" \
        --unset DYLD_LIBRARY_PATH --unset DYLD_FALLBACK_LIBRARY_PATH
    done
    for tool in ar ranlib; do
      makeWrapper "${stdenv.cc.bintools}/bin/$tool" "$out/libexec/hawk-tools/$tool" \
        --unset DYLD_LIBRARY_PATH --unset DYLD_FALLBACK_LIBRARY_PATH
    done
    wrapProgram "$out/bin/cargo-hawk" \
      --prefix PATH : "$out/libexec/hawk-tools:${rust}/bin" \
      --set CARGO "${rust}/bin/cargo" \
      --set RUSTC "${rust}/bin/rustc" \
      --set RUSTDOC "${rust}/bin/rustdoc" \
      --set CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER "$out/libexec/hawk-tools/cc" \
      --set CC "$out/libexec/hawk-tools/cc" \
      --set CXX "$out/libexec/hawk-tools/c++" \
      --set HOST_CC "$out/libexec/hawk-tools/cc" \
      --set HOST_CXX "$out/libexec/hawk-tools/c++" \
      --set CC_aarch64_apple_darwin "$out/libexec/hawk-tools/cc" \
      --set CXX_aarch64_apple_darwin "$out/libexec/hawk-tools/c++" \
      --set CC_AARCH64_APPLE_DARWIN "$out/libexec/hawk-tools/cc" \
      --set CXX_AARCH64_APPLE_DARWIN "$out/libexec/hawk-tools/c++" \
      --unset RUSTC_WRAPPER --unset RUSTC_WORKSPACE_WRAPPER
    runHook postInstall
  '';
  passthru = { inherit rust; };
  meta = {
    description = "Workspace visibility checks with a matching Rust analysis toolchain";
    homepage = "https://github.com/astral-sh/hawk";
    license = with lib.licenses; [
      asl20
      mit
    ];
    mainProgram = "cargo-hawk";
    platforms = [ "aarch64-darwin" ];
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
  };
}
