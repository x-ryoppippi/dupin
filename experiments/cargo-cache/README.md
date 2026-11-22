# Local Cargo and sccache experiment

`measure.nu` compares Cargo builds with and without sccache for one workspace
package. It uses a private temporary cache, a private sccache server socket, and
separate target directories. It leaves the temporary directory and targets in
place so the recorded run can be inspected; the output JSON records its path.

Run it from the repository root after dependencies are available locally:

```sh
nu experiments/cargo-cache/measure.nu /path/to/sccache \
  --package dupin-core --output /tmp/dupin-core-cache-results.json
```

The default package is `dupin`. Cargo uses `--offline`, so the experiment will
fail if the local Cargo registry or Git checkout cache does not already have
everything needed by that package. Build output is not launched.

Each run measures a native fresh target, a cold sccache fresh target, a warm
sccache build in a different fresh target directory, a native fresh-target
control, and two same-target Cargo reuse controls. It then removes only the two
target directories created by that run and rebuilds them at the same absolute
paths, once with a warm sccache and once without it. The cache itself is never
cleared between sccache cases; sccache statistics are reset for each measured
command.

The JSON records each full Cargo command, wall time, sccache statistics, cache
size, Cargo's `compiler-artifact` freshness flags, tool versions, source commit,
script and lockfile hashes, macOS version, hardware model, and whether the
worktree was dirty. Cargo artifact freshness describes Cargo's
target-directory decision; use sccache's hit and miss counters to identify
compiler cache replay.

This is a local debug-profile `cargo build` experiment. It does not measure the
Nix derivation, CI cache restore, release binary size, application launch, or
the effect of editing a source file. One run is exploratory evidence, not a
statistical comparison or a reason by itself to enable sccache by default.

The child processes inherit the caller's normal build environment except that
inherited `SCCACHE_*` variables and `CARGO_INCREMENTAL` are removed. The script
sets `RUSTC_WRAPPER` explicitly for each case and sets
`RUSTC_WORKSPACE_WRAPPER` to empty. This keeps the cache and server isolated and
lets Cargo use its default incremental setting.
