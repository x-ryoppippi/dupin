# Calculator query protocol fixture

This separate workspace exercises the [experimental query protocol](../../docs/plugin-protocol.md)
with a real calculator Wasm guest. It is not loaded by Dupin and changes no app
permissions, entitlements or runtime dependency. The shared protocol crate is
part of the root workspace so its validation tests run with the normal checks.

Build the guest using the repository's Rust toolchain and bare Wasm standard
library. Nix's temporary linker shell supplies `lld` without a global install:

```sh
nix shell --inputs-from . nixpkgs#lld --command cargo build \
  --manifest-path experiments/plugin-protocol/Cargo.toml --locked --release \
  -p dupin-protocol-calculator --lib --target wasm32-unknown-unknown
cargo run --manifest-path experiments/plugin-protocol/Cargo.toml --locked \
  -p dupin-protocol-host -- \
  experiments/plugin-protocol/target/wasm32-unknown-unknown/release/dupin_protocol_calculator.wasm \
  experiments/plugin-protocol/requests/compound.json
```

The host prints a validated JSON response. The checked-in requests also cover
arithmetic and a zero-limit pathological expression. The latter must return
an empty result without evaluating it. There are no clipboard, URL, network,
file-search or calendar operations in this fixture.
The calculator is stateless and rejects supplied saved state explicitly.

```sh
cargo test -p dupin-plugin-protocol
cargo test --manifest-path experiments/plugin-protocol/Cargo.toml --locked --workspace
cargo test --manifest-path experiments/plugin-protocol/Cargo.toml --locked \
  --release -p dupin-protocol-host
```

Host tests use adversarial WAT modules to check pointer bounds, overlap, output
lengths, malformed/foreign responses, imports, ABI types/version, fuel, memory,
tables and per-request teardown. The host enables Wasmi's `auto-dispatch` fallback
so the same infinite-loop tests terminate by fuel in both debug and release
builds. Disabling default Wasmi features without preserving that fallback made
the debug loop test overflow its native stack; release-only measurements did
not expose it.

This validates a query-only path. Manifest registration, action execution,
capability grants, cancellation/deadlines and the production renderer remain
required before this is a usable plugin platform.
