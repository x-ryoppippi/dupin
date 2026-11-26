{
  perSystem =
    {
      config,
      lib,
      pkgs,
      ...
    }:
    let
      expected = "pnpm@${config.packages.pnpm.version}";
      # Each pnpm project in the repository pins the pnpm the development shell provides.
      projects = [
        "website"
        "telemetry"
      ];
      pinned = project: (lib.importJSON ../${project}/package.json).packageManager;
    in
    {
      packages.pnpm = pkgs.pnpm_12;
      checks.pnpm-version = pkgs.runCommand "pnpm-version-check" { } ''
        ${lib.concatMapStrings (project: ''
          if [ '${pinned project}' != '${expected}' ]; then
            echo "ERROR: ${project}/package.json packageManager must be ${expected}, matching the flake." >&2
            exit 1
          fi
        '') projects}
        touch "$out"
      '';
    };
}
