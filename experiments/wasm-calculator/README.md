# Calculator guest experiment

This separate Cargo workspace exercises the runtime decision gate in
[the plugin proposal](../../docs/plugin-design.md). Dupin does not depend on it.
It is an experiment, not the plugin ABI or a runtime adoption decision.

The guest and native benchmark use the same pinned `fend-core` evaluation code.
The Wasm target has no WASI, host imports, clock or cooperative deadline. Wasmi
meters execution in the host with fuel and a 16 MiB linear-memory limit. The
benchmark checks result equality for arithmetic, length, speed and area; then
verifies a fuel trap and rejection of memory growth beyond the limit.

## Run

Use the repository's Rust toolchain (initial experiment: Rust 1.99.1 on
`aarch64-apple-darwin`) with the `wasm32-unknown-unknown` standard library and
`lld` available. Nix's toolchain may require the temporary linker shell below:

```sh
nix shell --inputs-from . nixpkgs#lld --command cargo build \
  --manifest-path experiments/wasm-calculator/Cargo.toml --locked --release \
  -p dupin-calculator-guest --lib --target wasm32-unknown-unknown
cargo run --manifest-path experiments/wasm-calculator/Cargo.toml --locked \
  --release -p dupin-wasm-calculator-bench -- \
  experiments/wasm-calculator/target/wasm32-unknown-unknown/release/dupin_calculator_guest.wasm
```

The runner prints JSON with guest bytes, module loading and instantiation times,
first evaluation and 1,000-iteration warm means, fuel use and limit assertions.
Wasmi uses explicit `LazyTranslation`: module loading validates code, while
translation occurs on first use and contributes to each query's first evaluation.
The first expression's first evaluation also sets up the guest context;
later expressions share that context. Native evaluations also share a context.
The Wasm loop includes host memory writes/reads and result allocation. Run with
an idle machine and retain toolchain, machine and build metadata alongside results;
single-run timing is exploratory rather than a latency guarantee.

An initial run is recorded in
[results/2026-11-22-aarch64-darwin.json](results/2026-11-22-aarch64-darwin.json),
including toolchain and source/artifact hashes. The baseline commit identifies
the app revision the experiment was added to; source hashes identify the fixture.

## Interpreter versus compiled execution

`compiled-host` runs the same guest and query cases with Wasmtime 51.0.0,
defaults disabled and only `cranelift`, `runtime` and `std` enabled. It explicitly
selects Cranelift rather than Pulley, which is another interpreter. The existing
Wasmi runner remains unchanged so the two paths can be examined independently.

On macOS, after building the bare guest above:

```sh
nu experiments/wasm-calculator/compare.nu \
  experiments/wasm-calculator/target/wasm32-unknown-unknown/release/dupin_calculator_guest.wasm
```

The script copies the manifests, lockfile, Rust sources and guest into a temporary
snapshot before building either host. It records their hashes, compiler and OS,
executable bytes, each runner's JSON, and macOS `time -l` process statistics.
Each host is built separately to avoid merging dependency features across the
two runners. Build output lives inside that run's directory, preventing another
invocation from replacing an executable before it is copied or measured.
Git state and the root nixpkgs pin are recorded with the toolchain.
`--prepare-only` checks snapshot preparation without building or executing.
The final `results.json` stays in the reported run directory. Both runners must
produce the same query/result pairs and pass fuel and memory-limit assertions.

Module loading includes native compilation in Wasmtime; Wasmi's lazy translation
instead contributes to first-use latency. Fuel counts have different meanings in
the two runtimes and must not be compared as equivalent work units. Cranelift
compilation is single-threaded here; parallel compilation and precompiled module
loading are not enabled or measured. Process peak
memory covers compilation, queries and the limit checks; it is not idle memory
or a bound on host allocations. Standalone executable sizes include the native
evaluator and are not application bundle deltas.

Cranelift produces executable memory. These unsigned command-line runs do not
prove compatibility with Dupin's hardened-runtime signing or justify entitlement
changes. A compiled runtime still needs an app-level size/signing measurement
before adoption. No app dependency, permission or entitlement changes are made.

The [recorded M5 Max run](results/2026-11-22-runtime-comparison-aarch64-darwin.json)
uses the same 766,406-byte guest. Both engines passed result equality, fuel cutoff
and exact memory-boundary checks.

| Measurement                |      Wasmi 2.0.2 | Wasmtime 51.0.0 / Cranelift |
| -------------------------- | ---------------: | --------------------------: |
| Module load / compilation  |         2.534 ms |                  373.450 ms |
| Warm `560 km to mi` mean   |         0.976 ms |                    0.117 ms |
| Warm `30*(60+1)` mean      |         0.131 ms |                    0.017 ms |
| Warm `60 mph in m/s` mean  |         1.118 ms |                    0.130 ms |
| Warm `1 acre in m^2` mean  |         0.986 ms |                    0.115 ms |
| Process peak footprint     | 23,167,360 bytes |            42,598,856 bytes |
| Standalone host executable |  2,091,392 bytes |             7,618,528 bytes |

Cranelift improves warm query time here while paying more at module load and in
process memory. These exploratory numbers do not decide production adoption;
the signed app, cold/idle costs and protocol costs still need consideration.

References: [Wasmtime compilation configuration](https://docs.rs/wasmtime/51.0.0/wasmtime/struct.Config.html),
[resource limits](https://docs.rs/wasmtime/51.0.0/wasmtime/struct.StoreLimitsBuilder.html),
[fuel traps](https://docs.rs/wasmtime/51.0.0/wasmtime/enum.Trap.html),
[Pulley](https://docs.wasmtime.dev/examples-pulley.html), and
[Apple's executable-memory entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.cs.allow-jit).

## Boundaries

The pointer-based exports are private to this trusted fixture. They do not define
safe ownership or validation for third-party plugins. A trapped instance is
discarded on process exit; production execution still needs cancellation,
generation routing, clean teardown and bounded protocol messages.

The linear-memory limit does not bound all host/runtime allocations. These
measurements exclude permission brokerage, currency rates and the view protocol.
The host links the native evaluator for comparison, so its executable size is
not a Wasmi-only size measurement. Measure stripped Dupin executables, signed app
bundles and shipped archives separately before adopting a runtime. A second guest
must also be measured to expose duplicated library costs.
