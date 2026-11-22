# Nix Cargo dependency cache experiment

The flake keeps separate compiled dependency outputs for the release binary and
the debug Clippy/test check, then bundles the binary independently of Rust
compilation. Both profiles use the existing vendored sources, audited
Cargo, compiler, target linker and frame-pointer setting. The non-flake
`default.nix` build remains available as the native baseline. `package.nix`
owns the shared vendor hash; the flake exposes that builder as `dupin.baseline`.

Both builders map their vendor source locations to `/vendor`. The baseline
uses cargoSetupHook's writable copy, while the cached builder maps its vendor
adapter. The release and debug dependency caches and their consumers share the
cached mapping. This keeps temporary build paths out of diagnostics and makes
the executable-size comparison independent of Nix's build-directory names.

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
timing edits. It roots both candidate dependency outputs, then inserts the same
unused Rust constant before each archived UI module's imports. A fresh UUID makes each run's
edited application outputs new, while the unused constant has no runtime effect.
Dependency output paths must stay identical and application/check
output paths must change. The four edited builds run sequentially and offline.

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
the measured dependency cache. CI cache reuse remains to be measured.

## Recorded source-edit run

[Results](results/2026-11-22-aarch64-darwin.json) compare native builder
`21612d4e38153b006ded83dfb9d5a2d2c33f39c3` with cache candidate
`999932b8d7eb0f0d16c32147751f62415f142c6d`. Both snapshots contain the same
Rust, packaging, lockfile and version inputs. The unused source edit forces new
application/check outputs while retaining both dependency-cache output paths.

| Source-edit build | Native builder | Cached builder |
| ----------------- | -------------: | -------------: |
| Release app       |        148.2 s |         76.0 s |
| Clippy and tests  |         77.9 s |         18.4 s |

The initial release and debug dependency builds took 93.3 s and 73.8 s. The
table measures subsequent edits with those outputs already available. This is
one sequential run on a shared machine, with uncontrolled load; it does not
establish repeatable timing improvements or CI cache performance.

Both edited executables contain exactly the same bytes: 18,537,328 bytes, with
SHA-256 `273262b710c29b5a500f406072ef9a59546b4933066d2ac3abede1f72c4623f2`.
Decoded dependency audits and runtime references match. The resource-only
Info.plist probe changes the app bundle output while preserving the binary and
both dependency outputs. Full provenance and output paths are in the record.
