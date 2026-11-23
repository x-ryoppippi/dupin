# Native JavaScript runtime experiment

This is the adoption gate for native QuickJS-ng through `rquickjs`, the first step
of the [extension platform rollout](https://github.com/x-ryoppippi/dupin/issues/218).
It measures a pinned Dupin revision, not the current working tree, and it does not
adopt the runtime: no Dupin crate depends on `rquickjs` after this experiment.

```sh
nix develop -c nu experiments/js-runtime/measure.nu --prepare-only
nix develop -c nu experiments/js-runtime/measure.nu
```

Run it on an Apple silicon Mac with Nix, Git and `patch`. The dev shell supplies
Cargo, Clang and Nushell for the bench. Three full app builds take several minutes
each. `--skip-size` runs only the bench and `--skip-bench` only the app builds.
`--samples` (default 2,000) sets the warm calls per workload and `--cold-runs`
(default 200) the fresh processes per cold mode. Results and app copies remain in
the reported temporary directory; `results.json` holds the measurements and build
provenance.

The pinned runtime is `rquickjs` 0.14.0, the current crates.io release, which
bundles QuickJS-ng 0.16.2. It is built with `default-features = false` and only
`std`, so neither `rust-alloc` nor `allocator` replaces QuickJS's own malloc
wrapper, which is what enforces the runtime memory limit.

## Signed app size

The default baseline is commit `e58a51417f0062c48a125fdb71797b2ee6e54470`, which
already links wasmi, so both sides of each comparison contain it. The script
extracts three `git archive` snapshots:

- the unchanged baseline;
- `js-oz`: [`probe.patch`](probe.patch) applied, with QuickJS compiled at `-Oz`,
  which the release profile's `opt-level = "z"` gives the C build;
- `js-o3`: the same patch plus a `rquickjs-sys` profile override that compiles
  QuickJS at `-O3`.

The patch adds an optional `js-runtime` feature to the `dupin` crate, enables it in
the Crane release build and refreshes `Cargo.lock` and the vendor hash. The feature
adds a `--dupin-js-runtime-probe` argument, handled before the CLI or the app
starts. It plays the part of a minimal adapter: it creates a runtime with a 16 MiB
heap, a 512 KiB stack and a 100 ms interrupt deadline, compiles the design's word
counter to bytecode, loads that bytecode in a fresh runtime, calls `search("two
words")` and reads the row back, then checks that the interrupt handler stops a busy
loop. Running the probe proves LTO kept the engine and that it works under the
hardened runtime.

Each package is copied, passed through the release packager's `unlink-store` helper,
ad-hoc signed with the hardened runtime, verified and zipped with `ditto`, as the
[Wasm size experiment](../wasm-app-size/README.md) does.

## Bench

[`bench`](bench) is a standalone crate, outside the workspace, built twice from the
same lock file: once with QuickJS at `-Oz` and once at `-O3`. Every invocation runs
on a worker thread with Rust's default 2 MiB stack, as provider workers do, and
follows the design: a fresh runtime and context with the limits above, a job pump
capped at 10,000 promise jobs and the deadline, host validation of the returned rows
(at most 200 rows, string fields of at most 1 KiB), then teardown. Each sample
includes all of that.

- **Warm**: 50 discarded calls, then 2,000 timed calls of a cached-bytecode module
  whose `search(query)` scores 400 embedded candidates, sorts them and returns 50
  rows ([`search.js`](bench/fixtures/search.js)). The word counter is timed the same
  way.
- **Cold**: 200 fresh processes per mode read the module from disk, either as cached
  bytecode or as source, and report the time from `main` to validated rows. The
  parent also records spawn-to-exit time.
- **Footprint**: a fresh process records its physical footprint while idle, with an
  empty and a loaded live context, after the first drop and across 1,000
  create/drop cycles, plus its lifetime peak.
- **Containment**: each case in [`fixtures/containment`](bench/fixtures/containment)
  runs in its own process. A case passes when it ends without crashing within the
  100 ms budget plus 10 ms, and the process's peak footprint stays within the 16 MiB
  heap limit plus 8 MiB for allocator and stack overhead. The parent kills a child
  after 10 s and also reports whether it ended within the design's two-second outer
  limit.

## Results

The recorded run is in
[`results/2026-11-23-aarch64-darwin.json`](results/2026-11-23-aarch64-darwin.json),
on an M5 Max with macOS 27.2.1. Both probes printed `js_runtime=ok title="2 words"`
and stopped the busy loop after 99 ms, without additional entitlements.

The Mac was shared with other build jobs throughout: the one-minute load average was
21–23 during the bench and 26–37 earlier. Latency figures are therefore pessimistic
and noisy. An earlier full run under the same conditions measured the `-Oz` warm p95
at 6.39 ms, a failure; its other gate verdicts were the same as below. Repeat the bench
on an idle machine before relying on the `-Oz` latency.

| Variant       | Signed app bytes | ZIP bytes | App increase | ZIP increase |
| ------------- | ---------------: | --------: | -----------: | -----------: |
| Baseline      |       15,479,843 | 9,138,733 |            — |            — |
| QuickJS `-Oz` |       16,063,683 | 9,491,297 |      583,840 |      352,564 |
| QuickJS `-O3` |       16,537,171 | 9,646,049 |    1,057,328 |      507,316 |

| Measurement (ms)                      | `-Oz` p50 / p95 / p99 | `-O3` p50 / p95 / p99 |
| ------------------------------------- | --------------------: | --------------------: |
| Warm 400-candidate search, whole call |    3.34 / 4.71 / 8.47 |    1.77 / 2.50 / 3.44 |
| … runtime and context setup           |    0.22 / 0.37 / 0.72 |    0.16 / 0.26 / 0.42 |
| … bytecode load and module evaluation |    1.17 / 1.70 / 3.25 |    0.66 / 0.92 / 1.35 |
| … `search` call                       |    1.49 / 2.19 / 4.06 |    0.63 / 0.84 / 1.06 |
| … row validation                      |    0.05 / 0.10 / 0.18 |    0.04 / 0.09 / 0.15 |
| … teardown                            |    0.30 / 0.51 / 0.83 |    0.24 / 0.43 / 0.68 |
| Warm word counter, whole call         |    0.32 / 0.90 / 1.19 |    0.11 / 0.25 / 0.46 |
| Cold first result, cached bytecode    |    3.66 / 7.08 / 8.29 |    1.72 / 2.06 / 2.24 |
| Cold first result, source             |    5.76 / 6.63 / 7.27 |    2.87 / 3.36 / 3.52 |
| Cold process spawn to exit, bytecode  | 10.34 / 16.17 / 18.80 |   6.58 / 7.38 / 10.43 |

The search module's bytecode is 57,508 bytes. In the footprint process, an idle
process measured 1.27 MiB; an empty context added 0.17 MiB (98 KiB of QuickJS
allocations) and a loaded context after one search 0.61 MiB in total (417 KiB of
QuickJS allocations). After the first drop the footprint was 0.08 MiB over idle.
Over 1,000 create/drop cycles it grew by 16 KiB at `-Oz` and 64 KiB at `-O3`, in
allocator-page steps, and the lifetime peak was 0.67 MiB over
idle. No idle context is kept alive.

| Containment case       | Outcome (both levels)                      | `-Oz` / `-O3` ms | Highest peak over idle |
| ---------------------- | ------------------------------------------ | ---------------: | ---------------------: |
| `busy-loop`            | interrupted                                |        102 / 100 |                0.2 MiB |
| `busy-module-body`     | interrupted during module evaluation       |        101 / 100 |                0.1 MiB |
| `busy-async-job`       | interrupted inside a promise job; deadline |        106 / 100 |                0.2 MiB |
| `heap-object-spike`    | out of memory (thrown value is `null`)     |          71 / 31 |               18.4 MiB |
| `heap-string-repeat`   | out of memory                              |            0 / 0 |                0.1 MiB |
| `heap-string-doubling` | `string too long`                          |            0 / 0 |                0.2 MiB |
| `heap-array-fill`      | out of memory                              |          17 / 10 |               10.9 MiB |
| `heap-typed-array`     | out of memory                              |            0 / 0 |                0.1 MiB |
| `stack-recursion`      | `Maximum call stack size exceeded`         |            1 / 0 |                0.6 MiB |
| `stack-json-parse`     | `Maximum call stack size exceeded`         |            8 / 4 |                2.0 MiB |
| `stack-json-stringify` | out of memory (thrown value is `null`)     |          68 / 26 |               18.9 MiB |
| `promise-chain-storm`  | job limit after 10,000 jobs                |           16 / 9 |                5.8 MiB |
| `promise-fanout-storm` | job limit after 10,000 jobs                |          44 / 18 |                6.3 MiB |
| `regexp-backtracking`  | interrupted inside `RegExp.prototype.exec` |        100 / 100 |                0.2 MiB |
| `native-string-search` | **not stopped; killed after 10 s**         |            — / — |                      — |
| `native-array-sort`    | out of memory during one native `sort`     |          45 / 18 |               13.7 MiB |

## Gates

| Gate (from the design)                            | `-Oz`                                                    | `-O3`               |
| ------------------------------------------------- | -------------------------------------------------------- | ------------------- |
| ≤1 MiB signed-app increase for engine and adapter | **Pass**, 0.56 MiB                                       | **Fail**, 1.008 MiB |
| Warm local pipeline p95 ≤5 ms                     | **Pass**, 4.71 ms (marginal: 6.39 ms in the earlier run) | **Pass**, 2.50 ms   |
| Warm local pipeline p99 ≤10 ms                    | **Pass**, 8.47 ms                                        | **Pass**, 3.44 ms   |
| First local result p95 ≤20 ms                     | **Pass**, ≤7.08 ms                                       | **Pass**, ≤3.36 ms  |
| All containment cases terminate                   | **Fail**                                                 | **Fail**            |

**The runtime does not pass the adoption gate as measured.** No single build passes
every gate, and containment fails at both levels.

- The interrupt handler stops busy loops in a function, in module evaluation and in
  promise jobs, and it does stop catastrophic RegExp backtracking: QuickJS-ng 0.16
  polls the same handler from its regular-expression engine. Heap, stack and
  job-pump limits end every allocation, recursion and promise-storm case, most within
  a few milliseconds.
- `String.prototype.indexOf` with a 2 MB needle over a 4 MB haystack runs a
  quadratic native loop that never polls the interrupt handler. It neither returned
  nor stopped within 10 s, so a hostile module can hold a worker thread indefinitely
  with a single call that fits easily within the heap limit. A native `sort` of
  300,000 values was also not interruptible; the heap limit ended it here, but a
  smaller array would only be bounded by its own run time. Containing these needs
  either engine patches that poll the interrupt in such loops, or a boundary the
  host can abandon (a killable helper process, or a thread the host stops waiting
  for, at the cost of its memory), before third-party packages can be enabled.
- The process footprint for the heap and JSON cases peaked at up to 18.9 MiB over
  idle, above the 16 MiB QuickJS limit, because the limit counts QuickJS's own
  allocations, not allocator rounding and fragmentation. The 8 MiB slack in the
  pass rule is an assumption of this experiment, not a design figure.
- Only `-Oz` fits the size budget, and its warm p95 sits near the 5 ms limit on a
  loaded machine. `-O3` halves the latency but exceeds 1 MiB by 8,752 bytes, before
  any real adapter code. The thrown value for an out-of-memory error can be `null`,
  so the host must classify failures itself rather than rely on the message.
- `Module::load` trusts its input: QuickJS bytecode is not validated, so a bytecode
  cache must be written only by the host from source it compiled, never accepted
  from a package.

This experiment does not cover Linux compatibility, 64 installed packages, broker
latency, keypress-to-frame timing or the incremental app footprint at idle, which
would mean launching Dupin itself. Ad-hoc signing does not reproduce Developer ID
signing, notarisation or stapling, and no DMG is measured.
