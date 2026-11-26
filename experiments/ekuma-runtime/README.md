# Ekuma runtime experiment

[Ekuma](https://github.com/hajimehoshi/ekuma) v0.1.0 is a JavaScript engine written in pure Go:
a bytecode interpreter with nearly all of ES2026, `Intl` and `Temporal`, under 0BSD. This
experiment asks whether it could replace QuickJS-ng as Dupin's guest runtime. It runs Ekuma
the way the [extension platform](../../docs/design/extension-platform.md) runs QuickJS-ng, in a
sandboxed helper process, and puts it through the gates of the
[QuickJS-ng adoption gate](../js-runtime/README.md). It does not adopt the runtime: nothing in
the Dupin workspace depends on Go or Ekuma.

```sh
nix shell --inputs-from . nixpkgs#go nixpkgs#nushell --command nu experiments/ekuma-runtime/measure.nu
```

Run it on an Apple silicon Mac with Nix. It needs Xcode's `cc`, `strip`, `codesign` and
`ditto`. `--skip-size` runs only the bench and `--skip-bench` only the builds; `--samples`
(default 2,000) and `--cold-runs` (default 100) match the QuickJS-ng helper measurement. The
script prints the path of `results.json`.

## Recommendation

**Keep watching; do not adopt.** Ekuma contains hostile guests better than QuickJS-ng and gives
guests `Intl` and `Temporal` for free, but it costs 7–16 times the size budget, its warm query
is 1.8 times slower than the QuickJS-ng helper, it has no memory limit, and its Go API is
unstable at a release made the day of this experiment. Revisit it if Ekuma gains a memory
limit and a compiled-module cache, or if Dupin decides guest `Intl` is worth about 16 MB.

| Gate or concern                            | QuickJS-ng helper (shipped)          | Ekuma helper                                                       |
| ------------------------------------------ | ------------------------------------ | ------------------------------------------------------------------ |
| Size (owner accepts about 1 MiB)           | +1,057,328 B signed app              | +7.8 MB, +8.2 MB with zones, +16.7 MB with `Intl`; a second binary |
| Warm 50-row query p95 ≤5 ms, p99 ≤10 ms    | 1.43 / 1.56 ms                       | 2.56 / 2.72 ms: passes, 1.8× slower                                |
| First result p95 ≤20 ms                    | 13.17 ms                             | 12.02 ms                                                           |
| Helper round trip p95 ≤1 ms                | not measured separately              | 0.06 ms                                                            |
| Footprint after 2,050 warm queries         | 6.20 MiB                             | 19.1 MiB (idle 8.1 MiB)                                            |
| Interrupt every CPU-bound fixture in place | no: `native-string-search` is killed | yes, and the helper serves the next call                           |
| Memory limit                               | 16 MiB runtime limit                 | none in the engine; a polling watchdog overshoots to 19–129 MiB    |
| `Intl` and `Temporal` for guests           | no                                   | yes, including `ja-JP` and `en-GB` formatting and named zones      |
| Bytecode cache                             | yes                                  | no: every run parses its source again                              |
| Toolchain                                  | Rust only                            | Go 1.27 beside Rust, a second vendor hash, second-binary signing   |
| API stability                              | `rquickjs` 0.14                      | v0.1.0 of 2026-11-26, "the Go API is unstable"                     |

The latency and footprint columns come from the same Blacksmith M4 Pro runner in one session.

## Helper

[`helper`](helper) is a Go counterpart of `dupin --extension-host`. Before it reads a request it
loads the local time zone, lowers the same resource limits and enters the same `(deny default)`
Seatbelt profile as [`sandbox.rs`](../../crates/dupin-extension-host/src/sandbox.rs), calling
`sandbox_init` through [purego](https://github.com/ebitengine/purego) so the build stays free of
cgo. It then answers one JSON line per request on standard input:

- `compile` parses a module. Ekuma has no bytecode format a host could keep, so the app would
  cache source and the helper would parse it on every run.
- `run` gives each call a fresh `ekuma.VM`, as each QuickJS-ng call gets a fresh runtime and
  context. It parses and evaluates the module, calls `search(query)`, settles a returned promise
  and validates the rows as the QuickJS-ng helper does (at most 200 rows, string fields of at
  most 1 KiB). A package sent once is held under its name, as the QuickJS-ng helper holds
  bytecode.
- `ping` answers at once, for the bare round trip.

A `context.Context` with the call's deadline covers module evaluation, the call, its promise
jobs and encoding the result. `Value.Get` on a guest value would run guest getters with no
deadline, so the helper encodes the rows with the guest's own `JSON.stringify` under the call's
context and validates plain JSON in Go. Ekuma has no heap limit, so a goroutine reads the Go
heap every millisecond and cancels the call once it has grown by 16 MiB since the call began.
That is sound only because a helper runs one call at a time. `debug.SetMaxStack` lowers Go's
1 GB goroutine stack limit to 64 MiB.

[`bench`](bench) drives the helper over pipes with a cleared environment from `/`, as Dupin
starts its helper. It measures what `latency_through_the_extension_host` in
[`crates/dupin/tests/extension_host.rs`](../../crates/dupin/tests/extension_host.rs) measures,
with the same 400-candidate module and query, and runs the
[js-runtime containment fixtures](../js-runtime/bench/fixtures/containment) through the helper
with a 100 ms budget, one fresh helper per case, then a word-count query to the same helper to
show it survived. A case passes as in the QuickJS-ng experiment: it ends within 110 ms and the
footprint stays within 24 MiB over idle. [`features.js`](bench/features.js) probes what guests
would use `Intl` and `Temporal` for.

## Results

Recorded runs:
[`results/2026-11-26-blacksmith-m4-pro.json`](results/2026-11-26-blacksmith-m4-pro.json), on a
Blacksmith `blacksmith-6vcpu-macos-latest` runner (Apple M4 Pro, virtual, macOS 26.3, load
average 2–4), and
[`results/2026-11-26-aarch64-darwin.json`](results/2026-11-26-aarch64-darwin.json), on an M5 Max
with macOS 27.2.1 (load average 4–6). Both used Go 1.27.3 from the flake's nixpkgs and the
`intl,timetzdata` helper. The QuickJS-ng figures are `cargo test --release -p dupin --test
extension_host -- --ignored latency_through_the_extension_host` on the same machine, run just
before. The bench is Go too; it turns off its own collector, so its GC adds nothing to the
helper's timings.

### Latency

| Measurement (ms, p50 / p95 / p99)     | QuickJS-ng, Blacksmith |     Ekuma, Blacksmith |    QuickJS-ng, M5 Max |         Ekuma, M5 Max |
| ------------------------------------- | ---------------------: | --------------------: | --------------------: | --------------------: |
| Compile in a helper of its own        |  11.15 / 12.89 / 14.07 | 10.18 / 10.95 / 11.39 | 11.84 / 13.12 / 16.82 |  9.85 / 10.42 / 11.11 |
| Cold first result in a fresh helper   |  11.82 / 13.17 / 13.80 | 11.35 / 12.02 / 13.43 | 11.12 / 11.60 / 11.80 | 10.91 / 11.81 / 13.42 |
| … of which spawn, sandbox and hello   |                      — |    8.47 / 8.98 / 9.53 |                     — |   8.81 / 9.55 / 11.34 |
| Warm 400-candidate search, round trip |     1.19 / 1.43 / 1.56 |    1.89 / 2.56 / 2.72 |    0.87 / 0.96 / 1.14 |    1.43 / 2.17 / 2.43 |
| Warm search with `GOGC=400`           |                      — |    1.34 / 3.00 / 3.31 |                     — |    1.03 / 2.36 / 2.97 |
| Warm word counter, round trip         |                      — |    0.20 / 0.57 / 0.64 |                     — |    0.13 / 0.35 / 0.41 |
| Empty `ping` round trip               |                      — |    0.03 / 0.06 / 0.07 |                     — |    0.01 / 0.01 / 0.02 |

The cold rows are not quite like for like: QuickJS-ng's cold first result loads cached bytecode,
while Ekuma's parses source, and the QuickJS-ng helper is the whole 15 MB `dupin` executable.
Ekuma's compile is a parse whose result is thrown away.

Inside the helper, the median warm search on Blacksmith spent 0.15 ms creating the VM, 1.04 ms
parsing and evaluating the 53 KB module, 0.29 ms in `search` and 0.06 ms encoding and
validating. Ekuma ran `search` itself about twice as fast as QuickJS-ng at `-O3` did in process
(0.63 ms p50 in the js-runtime experiment), but parsing on every call, which QuickJS-ng skips
with bytecode, costs more than that saves.

**GC.** At Go's default `GOGC=100`, a collection overlapped 1,741 of the 2,000 warm calls on
Blacksmith (1,451 on the M5 Max). Calls during a collection were about 0.5 ms slower at p99
(2.75 against 2.21 ms on Blacksmith, 2.51 against 2.02 ms on the M5 Max), so the collector,
not pauses, sets the tail, and p99 stays well inside the 10 ms gate. `GOGC=400` cut
collections to 329 calls and the median to 1.34 ms, but each collection was larger: p99 rose
to 3.31 ms and the footprint doubled to 38.6 MiB.

### Memory

| Footprint (MiB)                           |    Blacksmith |        M5 Max |
| ----------------------------------------- | ------------: | ------------: |
| Idle helper after its hello               |          8.14 |          8.95 |
| After the first search                    |         11.94 |         12.53 |
| After 2,051 searches                      |         19.11 |         20.89 |
| After 3,051 searches, now / lifetime peak | 18.64 / 20.00 | 20.70 / 21.16 |

The QuickJS-ng helper measured 6.20 MiB after the same warm queries on Blacksmith and 6.73 MiB
on the M5 Max. Ekuma's footprint is bounded by Go's collector, not by dropping each runtime:
garbage from finished calls stays until the next collection. Ekuma offers no limit on memory or
stack and no statistics of a VM's memory; its README lists both as missing. Its only caps are
fixed: strings of at most 1 GiB and array buffers of at most 4 GiB on 64-bit platforms.

### Containment

Four runs: each machine at `GOGC=100` and `GOGC=400`.

| Case                   | Outcome in every run                           |    ms | Peak over idle (MiB) |
| ---------------------- | ---------------------------------------------- | ----: | -------------------: |
| `busy-loop`            | interrupted by the context                     |   100 |              0.5–1.9 |
| `busy-module-body`     | interrupted during module evaluation           |   100 |              0.6–1.9 |
| `busy-async-job`       | interrupted inside a promise job               |   100 |              0.6–1.9 |
| `heap-object-spike`    | stopped by the heap watchdog                   | 10–15 |            18.5–19.7 |
| `heap-string-repeat`   | stopped by the heap watchdog                   |   1–2 |             3.1–15.2 |
| `heap-string-doubling` | stopped by the heap watchdog                   |   2–6 |        20.2–**38.4** |
| `heap-array-fill`      | stopped by the heap watchdog                   |  4–19 |        19.5–**26.2** |
| `heap-typed-array`     | 1 GiB array returned (3 runs) or stopped (1)   |   1–2 |              3.2–3.6 |
| `stack-recursion`      | `RangeError: maximum call stack size exceeded` |   1–3 |              5.1–7.0 |
| `stack-json-parse`     | `SyntaxError` after reading the whole input    | 12–23 |           **95–129** |
| `stack-json-stringify` | stopped by the heap watchdog                   |   7–9 |        21.1–**24.5** |
| `promise-chain-storm`  | stopped by the heap watchdog                   |  7–13 |            19.4–20.8 |
| `promise-fanout-storm` | stopped by the heap watchdog                   |  7–19 |            23.1–23.8 |
| `regexp-backtracking`  | interrupted inside the RegExp engine           |   100 |              0.6–1.9 |
| `native-string-search` | returned `-1`                                  |    14 |            17.0–17.9 |
| `native-array-sort`    | stopped by the heap watchdog                   | 16–34 |            19.3–21.1 |

Every case ended within the 100 ms budget without the helper crashing, and every helper
answered the next query. Ekuma polls its context inside native loops, and its
`String.prototype.indexOf` is linear, so `native-string-search`, which never returned under
QuickJS-ng and is why Dupin needs a killable helper, finishes in 14 ms. The rule's failures, one
to three cases per run and different cases each run, are all the 24 MiB footprint bound:

- The watchdog polls, so an allocation burst overshoots before the next poll and before the
  engine next checks the context.
- `JSON.parse` of a megabyte of `[` builds about 80 MiB of nested arrays before it reports the
  unterminated input, and does not stop for the cancelled context on the way. QuickJS-ng ends
  the same input with a stack overflow at 2 MiB.
- A single allocation below Ekuma's fixed caps happens at once. The 1 GiB `Uint8Array` stayed
  cheap in the recorded runs only because its pages were never touched; in an exploratory run
  of the same case the footprint reached 1,028 MiB before the watchdog stopped the call.

So the helper would still need Dupin's footprint watchdog and kill, as now, only for memory
instead of time. Parsing takes no context in Ekuma, as in QuickJS-ng, so compile-time fixtures
are bounded only by the outer kill in both.

### Features

With the `intl` package, a sandboxed helper formatted
`2026年11月26日木曜日 18:30` (`ja-JP`, `Asia/Tokyo`), `Thursday, 26 November 2026 at 09:30`
(`en-GB`, `Europe/London`), `明日`, `£1,234.50`, segmented `今日|は|良い|天気|です`, sorted kana
with `Intl.Collator('ja')`, added a month to `2026-01-31` as `2026-02-28` and wrote the year in
the Japanese calendar as `令和8年`. The calendar extension could format its own dates. Three
things needed the host:

- Named zones come from `/usr/share/zoneinfo`, which the sandbox denies, so the helper is built
  with Go's `timetzdata` tag, which embeds the zone database (+0.4 MB).
- Each VM resolves the name of the local zone by reading `/etc/localtime` and falls back to UTC
  in the sandbox, so the helper resolves it before `sandbox_init` and sets it on every VM.
- Ekuma asks CoreFoundation for the default locale, which inside the sandbox cannot reach the
  user's preferences and answers `en-US`, so Dupin would pass its locale, as the helper's
  `-locale` flag does.

The first `Intl` use in a helper took 15–25 ms while Ekuma decoded its data, on top of the cold
start. Modules load through a host-supplied synchronous loader (`VM.SetModuleLoader`), which
would map relative package imports and the `dupin` module as the QuickJS-ng loader does.
TypeScript would still be erased by Dupin; Ekuma parses JavaScript only.

### Size

| Build (darwin/arm64, `-trimpath -ldflags '-s -w'`) | Stripped bytes | Ad-hoc signed |  ZIP bytes |
| -------------------------------------------------- | -------------: | ------------: | ---------: |
| Helper                                             |      7,807,954 |     7,762,704 |  2,985,810 |
| Helper with `timetzdata`                           |      8,220,770 |     8,173,120 |  3,086,407 |
| Helper with `intl` and `timetzdata`                |     16,664,034 |    16,567,328 | 10,207,201 |

QuickJS-ng at `-O3` added 1,057,328 B to the signed app and 507,316 B to its ZIP. The helper
without `Intl` is 7.4 times that, and with `Intl` 15.7 times, a ZIP larger than the whole Dupin
app's (9.1 MB). Ekuma's README gives the same proportions for a hello-world program. Since
QuickJS-ng would go, the net app change would be about +6.8 MB, or +15.6 MB with `Intl`.

The helper would be a second executable in `Dupin.app`, against the design's rule that the
`dupin` executable re-runs itself as the helper. The other way is to build Ekuma with
`-buildmode=c-archive` and link it into `dupin`. [`carchive`](carchive) does that with a C
stand-in for `dupin`, linked with `-dead_strip` and stripped:

| c-archive         | Archive bytes | Increase of the linked executable |
| ----------------- | ------------: | --------------------------------: |
| Ekuma             |    18,217,264 |                         6,588,768 |
| Ekuma with `intl` |    29,520,392 |                        15,689,136 |

It links and runs, and saves little size over a second binary, but it is not practical for
Dupin:

- A c-archive starts the Go runtime from a load-time constructor, so its threads, heap and
  collector would run in the Dupin app process too, which never runs guests, as well as in the
  helper.
- Go installs handlers for synchronous signals such as `SIGSEGV`, `SIGBUS` and `SIGPROF` and
  relies on them for nil-pointer panics and preemption. Rust's runtime only installs its
  stack-overflow handler when none is set, so Go's would win and Rust's stack-overflow report
  would be lost; other native code that installs signal handlers without `SA_ONSTACK` can crash
  the Go runtime.
- Only one Go runtime may exist in a process, and Go cannot unload. A Go panic that escapes a
  call aborts the whole process.

Linking keeps one binary in name but puts a second language runtime in every Dupin process;
shipping the helper separately keeps Go out of the app but breaks the single-binary rule and
needs its own signing, notarisation and update path.

### Toolchain and licences

- Ekuma's `go.mod` requires Go 1.27; the flake's nixpkgs has 1.27.3, a 229 MiB closure. It would
  be built with `buildGoModule` and a `vendorHash`, beside Crane's Cargo vendor hash, and a
  c-archive would be passed to the Rust build as a separate derivation.
- With `CGO_ENABLED=0` it cross-compiles with no extra toolchain: the `intl,timetzdata` helper
  built for darwin/amd64 (17.2 MB), linux/arm64 (16.4 MB) and linux/amd64 (16.9 MB). The
  c-archive needs cgo and a C toolchain for each target.
- Two clean builds (`-a`, one with an empty `GOCACHE`) were byte-identical on each machine. The
  two machines' builds differed because Go stamps the VCS revision and dirty flag into the
  binary; a Nix build would pass `-buildvcs=false`.
- Ekuma is 0BSD, but every build embeds Unicode data under the Unicode License v3, whose notice
  must ship. With `intl`, the CLDR, CLDR collation and ICU calendar notices, three MIT calendar
  table notices and four ICU word-break dictionary notices must ship too. purego is
  Apache-2.0, and the Go runtime and standard library are BSD-3-Clause.

This experiment does not cover Linux confinement, Developer ID signing or notarisation of a
second binary, keypress-to-frame timing, or 64 installed packages.
