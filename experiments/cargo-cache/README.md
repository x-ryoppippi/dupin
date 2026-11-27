# Local Cargo and sccache experiment

`measure.nu` compares Cargo builds with and without sccache for one workspace
package. It uses a private temporary cache, a private sccache server socket, and
separate target directories. It leaves the temporary directory and targets in
place so the recorded run can be inspected; the output JSON records its path.

Run it from the repository root after dependencies are available locally:

```sh
nix build --inputs-from . nixpkgs#sccache --out-link /tmp/dupin-cache-sccache
nu experiments/cargo-cache/measure.nu /tmp/dupin-cache-sccache/bin/sccache \
  --package dupin --output /tmp/dupin-cache-results.json
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

`elapsed` is in nanoseconds; cache sizes are bytes.

This is a local debug-profile `cargo build` experiment. It does not measure the
Nix derivation, CI cache restore, release binary size, application launch, or
the effect of editing a source file. One run is exploratory evidence, not a
statistical comparison or a reason by itself to enable sccache by default.

The child processes inherit the caller's normal build environment except that
inherited `SCCACHE_*` variables and `CARGO_INCREMENTAL` are removed. The script
sets `RUSTC_WRAPPER` explicitly for each case and sets
`RUSTC_WORKSPACE_WRAPPER` to empty. This isolates the cache and server without
imposing an incremental profile override.

Explicit profile overrides such as `CARGO_PROFILE_DEV_INCREMENTAL` and the
caller's Cargo configuration still apply. Normal completion and caught command
failures stop the private server; interrupting a build may leave it running
until its idle timeout.

## Recorded run

[The recorded output](results/2026-11-22-aarch64-darwin.json) was generated from
clean commit `4d208c00a2cd76e5a0e741adc6ff5700680e2487` on macOS 27.2.1,
hardware model `Mac17,7`, with Rust 1.99.1 and sccache 0.19.0. The launcher,
dependencies and Nix configuration are unchanged from `45bc4a8`.

| Case                                                | Seconds | Rust hits | Rust misses |
| --------------------------------------------------- | ------: | --------: | ----------: |
| Native, fresh target                                |  28.403 |         — |           — |
| sccache, cold cache, fresh target                   |  39.405 |         0 |         210 |
| sccache, warm cache, different fresh target path    |  28.081 |       173 |          37 |
| Native, fresh-target control                        |  27.935 |         — |           — |
| sccache, existing target                            |   0.170 |         — |           — |
| Native, existing target                             |   0.238 |         — |           — |
| sccache, warm cache, same target path after removal |   7.686 |       210 |           0 |
| Native, same target path after removal              |  36.206 |         — |           — |

The existing-target cases issued no compiler requests. The final cache size was
483,011,452 bytes under the 2 GiB cap. Workspace incremental builds, procedural
macros and executables remain outside the Rust cache; the raw output records
the non-cacheable requests separately.

This sequential run shows that many cache hits can coexist with little change
in build time when the target path changes. Rebuilding at the same path had a
large benefit here: 7.686 seconds was also below both fresh native builds
(27.935 and 28.403 seconds). The slower 36.206-second native rebuild has no
established explanation. Local machine load and operating-system caches were not
controlled, so these are observations, not stable speedup estimates. They do
not establish the exact cause of each miss or show that path settings would
resolve it. The development shell and CI keep their existing
configuration. Sharing compiled Nix dependency outputs remains a separate
experiment tracked by [#95](https://github.com/x-ryoppippi/dupin/issues/95).

## Testbox run

[A second run](results/2026-11-27-testbox-aarch64-darwin.json) on a Blacksmith
Testbox (`VirtualMac2,1`, 12 CPUs, macOS 27.0) from `983024e`, with Rust 1.99.1
and sccache 0.19.0, used the same script unchanged:

| Case                                                | Seconds | Rust hits | Rust misses |
| --------------------------------------------------- | ------: | --------: | ----------: |
| Native, fresh target                                |  47.038 |         — |           — |
| sccache, cold cache, fresh target                   |  53.875 |         0 |         296 |
| sccache, warm cache, different fresh target path    |  47.486 |       226 |          70 |
| Native, fresh-target control                        |  47.388 |         — |           — |
| sccache, existing target                            |   0.220 |         — |           — |
| Native, existing target                             |   0.297 |         — |           — |
| sccache, warm cache, same target path after removal |  21.891 |       296 |           0 |
| Native, same target path after removal              |  47.289 |         — |           — |

A warm cache no longer saves anything in a target directory at a new path: 70
of 296 compilations still miss there, and the build takes as long as one
without sccache. Rebuilding the same path after removal still saves about 25
seconds, and a cold cache costs about 7 seconds more than building without it.
