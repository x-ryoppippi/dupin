# App-level Wasm size experiment

> **Historical.** Dupin no longer links wasmi: it was removed with the Wasm guest path once JavaScript extensions ran in the sandboxed extension host ([rollout](../../docs/design/extension-platform.md#rollout)). The script measures pinned revisions, so it still runs, but its results describe a runtime Dupin no longer ships.

This measures a pinned Dupin revision, not the current working tree. It applies a
measurement-only patch to disposable `git archive` snapshots and leaves the app's
normal startup, permissions and local configuration untouched.

```sh
nu experiments/wasm-app-size/measure.nu /path/to/calculator.wasm --prepare-only
nu experiments/wasm-app-size/measure.nu /path/to/calculator.wasm
```

Use a bare guest built by the calculator experiment, with its private allocation
and evaluation exports. The initial reference guest is `fend-core` 1.5.9. The
script records the actual guest bytes and SHA-256, so differently built guests
are distinguishable. The fixture is trusted; this is not third-party plugin
installation or a public ABI.

The default baseline is commit `ac3991952f90435ea207e7c0bd9f876dec371028`.
The baseline's flake pins the package toolchain, fonts and packaging tools.
Run on an Apple silicon Mac with Nix, Nushell, Git and `patch`. Full builds can
take several minutes. `--prepare-only` validates source extraction and patching
without compiling the application.

The variants are:

- unchanged Dupin;
- Wasmi 2.0.2 with `std`, `validate` and `auto-dispatch`, fuel enabled and a 16 MiB linear-memory
  limit, loading an empty module;
- the runtime plus a minimal external guest that returns 42;
- the runtime plus the calculator and minimal guests as separate resources.

Each variant is stripped through the package's existing Nix rules. The script
copies each app, uses the release packager's shared helper to replace Nix libiconv
links with the system library, ad-hoc signs it with the hardened runtime, verifies its signature
and creates the same ZIP format used by the updater. It records raw and signed
executable bytes, the sum of regular-file bytes in the signed app, and ZIP bytes.
It then invokes explicit probe arguments before GPU prewarming or normal app
startup, verifying that runtime/guest code was retained by LTO and executes.
Results and app copies remain in the reported temporary directory; `results.json`
contains structured measurements and build provenance.
Both Nix outputs have GC roots in that directory so a later build cannot collect
the earlier package. Removing the run directory releases those roots.

The patch saves the runtime executable before building the guest variants and
restores it before Nix's install hook. Otherwise the last linked variant would be
installed under the runtime-only label.

## Interpretation

The recorded Apple silicon run is in
[`results/2026-11-22-aarch64-darwin.json`](results/2026-11-22-aarch64-darwin.json).
All three signed-app probes passed, without additional entitlements.
This historical run used the patch before `auto-dispatch` was enabled. Its
recorded patch hash and measurements remain unchanged; the current patch needs a
new full run before claiming its bundle sizes.

| Variant                              | Signed app bytes |  ZIP bytes | App increase | ZIP increase |
| ------------------------------------ | ---------------: | ---------: | -----------: | -----------: |
| Baseline                             |       18,423,525 |  9,557,772 |            — |            — |
| Runtime                              |       19,559,797 |  9,954,186 |    1,136,272 |      396,414 |
| Runtime + minimal guest              |       19,576,474 |  9,957,490 |    1,152,949 |      399,718 |
| Runtime + calculator + minimal guest |       20,326,614 | 10,211,718 |    1,903,089 |      653,946 |

The calculator variant adds about 1.81 MiB to the app and 0.62 MiB to the ZIP.
Its executable is 144 bytes smaller than the runtime-only variant; whole-program
linking can change layout, so this is not a negative guest-code cost. Guest
resources are counted separately in the app total. Recorded artifact paths are
relative to the run directory.

These are deltas against the pinned app. They include the host probe code and
external resources, rather than treating a standalone interpreter as the app's
incremental cost. The minimal guest bundled alongside the calculator has no
duplicated feature libraries; two realistic feature guests remain a separate
measurement requirement.

Ad-hoc signing verifies hardened-runtime compatibility without asking for signing
credentials. It does not reproduce Developer ID certificates, notary submission or
stapling. ZIP metadata and signing can vary between runs. No DMG or memory/latency
measurement is included here; the calculator experiment covers execution timing
and limits separately. These results alone do not adopt a runtime.
