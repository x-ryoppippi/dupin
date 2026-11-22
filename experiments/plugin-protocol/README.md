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
It uses fend's search preview; when that preview is empty, the response contains
no items rather than an error. Final calculator evaluation policy belongs to #17.

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

`query_controlled` accepts a host-owned cancellation predicate and absolute
deadline. Exported calls run in resumable 50,000-fuel slices, checking control
before each call/resumption and after execution. Refills spend the original
1M startup or 20M query budget; they never reset that total. An operation
whose fuel cost exceeds one slice can receive the larger amount it requires,
provided the total budget permits it. Allocation limits still apply.
Modules compile eagerly during loading: Wasmi's lazy translation can consume
fuel without yielding a resumable invocation, so it must finish before these
small execution slices begin. This changes the fixture's loading policy and
does not replace the historical lazy-translation benchmark measurements.

The convenience `query` method used by the CLI has a five-second deadline and
no external cancellation signal. This is a fixture default, not a production
launcher latency budget. Cancellation, deadline and fuel exhaustion have
distinct payload-free errors; a stopped invocation drops its store.

This is cooperative control, not a hard real-time limit. Module loading is
outside the query deadline. Instantiation/start is checked before and after
and bounded by startup fuel; Wasmi does not expose a resumable start
through this linker API. Host parsing/translation, allocation and a single
charged operation cannot be interrupted by these checks. Startup and host
resource limits still need review before external modules run in the app.

This validates a query-only path. Manifest registration, action execution,
capability grants, production cancellation/deadline integration and rendering remain
required before this is a usable plugin platform.
