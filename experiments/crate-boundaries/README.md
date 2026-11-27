# Crate boundary and edit rebuild experiment

Two scripts measure how much a representative edit rebuilds. Both edit one file,
build, then restore it; they never commit or launch the app.

```sh
nu experiments/crate-boundaries/measure.nu --output /tmp/edits.json
CARGO_INCREMENTAL=0 nu experiments/crate-boundaries/measure.nu --repeat 1 \
  --edits 'function body|docs' --output /tmp/edits-non-incremental.json
nu experiments/crate-boundaries/measure.nu --repeat 1 --commands release \
  --edits 'function body' --output /tmp/edits-release.json
nu experiments/crate-boundaries/nix.nu --output /tmp/edits-nix.json
```

`measure.nu` drives Cargo in an existing target directory (`target` by
default). It first runs every selected command twice and records whether the
second run rebuilt anything, which shows whether `cargo build`, `cargo test`
and `cargo clippy` invalidate each other's units. Each edit is then measured
`--repeat` times; after each measurement the file is restored and the same
command runs again unmeasured, so the next edit starts from a built tree. The
JSON lists the units Cargo rebuilt and any third-party unit among them.

An edit either appends an unused constant ("new item") or changes a string
literal inside a function ("function body"), which forces that function's code
to be generated again. The docs edit appends an HTML comment to
`docs/development.md`.

`nix.nu` times `checks.aarch64-darwin.{lint-and-test,hawk,dupin}` one at a
time after each edit, from a store that already holds the unedited outputs,
and records which derivations Nix planned to build. It also bumps `VERSION`,
which tagpr does for every release.

## Recorded run

All runs used a Blacksmith Testbox (`blacksmith-12vcpu-macos-27`, 12 CPUs,
macOS 27.0) at `983024e`, with Rust 1.99.1 and the dependency outputs already
named 0.0.0.
One machine and one sequence; the runner's load was not controlled.

### Cargo, local development

Median of three runs, in seconds
([incremental](results/2026-11-27-cargo-incremental.json)). The
non-incremental and release columns are single runs with `CARGO_INCREMENTAL=0`
([non-incremental](results/2026-11-27-cargo-non-incremental.json)) and
`--release` ([release](results/2026-11-27-cargo-release.json)), for function
body edits.

| Edit                          | `build -p dupin` | `test --no-run` | `clippy` | non-incremental build | release build |
| ----------------------------- | ---------------: | --------------: | -------: | --------------------: | ------------: |
| dupin UI (`ui/keys.rs`)       |              2.9 |             3.1 |      1.4 |                   8.5 |         126.1 |
| dupin-core session (`search`) |              3.6 |             4.9 |      2.8 |                  10.6 |         129.2 |
| dupin-mac (`apps.rs`)         |              3.2 |             4.5 |      2.3 |                   9.5 |         126.8 |
| docs only                     |              0.2 |             0.2 |      0.3 |                   0.2 |             — |

Each edit rebuilt only the edited crate and the workspace crates above it.
No third-party unit was rebuilt, and alternating `build`, `test` and `clippy`
rebuilt nothing, so no feature difference between those commands forces
rebuilds. Appending an item and editing a function body cost the same.

### Nix checks

Seconds per check, each built alone
([results](results/2026-11-27-nix-checks.json)):

| Edit               | lint-and-test | hawk | dupin (release app) |
| ------------------ | ------------: | ---: | ------------------: |
| dupin UI           |          68.6 | 15.8 |               144.6 |
| dupin-core session |          68.1 | 15.9 |               144.6 |
| dupin-mac          |          68.5 | 16.0 |               144.3 |
| docs only          |           0.1 |  0.1 |                 0.1 |
| `VERSION` bump     |          67.7 | 15.8 |               144.5 |

Docs-only edits build nothing. The dependency outputs, Hawk's included, were
reused for every edit, including the `VERSION` bump.

## Reading

Editing deeper in the graph costs a few seconds more, but no split of the
`dupin` crate can save more than the 2.9 seconds an edit there takes now
incrementally. Release builds take 126–129 seconds whichever crate changes:
fat LTO with one codegen unit recompiles the whole program after any edit,
so compiling fewer crates first barely shortens it. Nix builds the workspace
without incremental compilation, where the gap between the cheapest and the
dearest edit is about two seconds.

Hawk reuses its own dependency output, so it takes about 16 seconds after any
of these edits. Building all three checks at once after the UI edit, as CI's
`build` job does, took 163 seconds, 19 more than the release app alone, since
the other two build alongside it.
