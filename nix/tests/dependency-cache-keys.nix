# Fails evaluation when a release version bump would give any dependency-only output a new
# store path, including ones buildPackage creates when given no cargoArtifacts.
{
  pkgs,
  crane,
  native,
  cargo-hawk,
}:
let
  artifacts =
    version:
    (import ../cargo-artifacts.nix {
      inherit pkgs crane cargo-hawk;
      native = native // {
        passthru = native.passthru // {
          commonArgs = native.passthru.commonArgs // {
            inherit version;
          };
        };
      };
    }).passthru;
  dependencyOutputs = dupin: {
    release = dupin.releaseArtifacts.outPath;
    debug = dupin.debugArtifacts.outPath;
    hawk = dupin.hawkArtifacts.outPath;
    latency = dupin.latencyBinary.cargoArtifacts.outPath;
  };
  current = dependencyOutputs (artifacts native.passthru.commonArgs.version);
  bumped = dependencyOutputs (artifacts "999.0.0");
in
assert current == bumped;
pkgs.runCommand "dupin-dependency-cache-keys" { } "touch $out"
