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
