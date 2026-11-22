# Nix Cargo dependency cache experiment

The flake keeps separate compiled dependency outputs for the release binary and
the debug Clippy/test check, then bundles the binary independently of Rust
compilation. Both profiles use the existing vendored sources, audited
Cargo, compiler, target linker and frame-pointer setting. The non-flake
`default.nix` build remains available as the native baseline and owns the shared
vendor hash through `dupin.baseline`.

Run the comparison from the repository with committed changes:

```sh
nix build --inputs-from . nixpkgs#rust-audit-info --out-link /tmp/dupin-rust-audit-info
nu experiments/nix-cargo-cache/measure.nu /tmp/dupin-rust-audit-info/bin/rust-audit-info \
  --baseline-revision <original-build-commit> --revision <cache-build-commit> \
  --output /tmp/dupin-nix-cache-results.json
```

The two revisions must contain identical Cargo manifests, lockfile, Rust code,
shader examples, packaging inputs and version, and pin the same nixpkgs. The
runner checks that the audited Cargo and compiler store paths match. It creates
private source archives and GC roots under `/private/tmp`; it never edits the
working tree or launches either application.

The runner builds the original app/checks and the candidate app/checks before
timing edits. It roots both candidate dependency outputs, then appends the same
unused Rust constant to each archived UI module. A fresh UUID makes each run's
edited application outputs new, while the unused constant has no runtime effect.
Dependency output paths must stay identical and application/check
derivations must change. The four edited builds run sequentially and offline.

An additional evaluation-only probe changes the bundle's Info.plist template.
It requires the bundle output path to change while the binary and dependency
output paths stay identical, then restores the template before timing builds.
Output paths reflect Nix's fixed-output dependency normalization; derivation
paths can change when the vendor input's source changes without requiring a
new dependency output.

The JSON records preparation separately from edited-build measurements, elapsed
nanoseconds, exact commands, derivations, outputs and compiler identities. It
also compares executable size, decoded audit metadata and runtime store
references. Source archives, build logs and GC roots remain in the reported
temporary directory for inspection.

One run is exploratory evidence. Machine load and operating-system caches are
not controlled. This measures Nix source-edit reuse, rather than a fresh-store
download or a CI cache restore. It does not measure application startup or
interact with the running launcher. The roots prevent local GC from removing
the measured dependency cache; CI already keeps derivations and outputs through
its existing Nix setup action.
