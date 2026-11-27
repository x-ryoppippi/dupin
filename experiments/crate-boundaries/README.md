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

All runs used a Blacksmith Testbox (`blacksmith-6vcpu-macos-latest`, 6 CPUs,
macOS 26.3) at `5f1ea19`, with Rust 1.99.1 and the dependency outputs already
named 0.0.0.
One machine and one sequence; the runner's load was not controlled.

### Cargo, local development

Median of three runs, in seconds
([incremental](results/2026-11-23-cargo-incremental.json)). The
non-incremental and release columns are single runs with `CARGO_INCREMENTAL=0`
([non-incremental](results/2026-11-23-cargo-non-incremental.json)) and
`--release` ([release](results/2026-11-23-cargo-release.json)), for function
body edits.

| Edit                          | `build -p dupin` | `test --no-run` | `clippy` | non-incremental build | release build |
| ----------------------------- | ---------------: | --------------: | -------: | --------------------: | ------------: |
| dupin UI (`ui/keys.rs`)       |              1.8 |             1.8 |      0.8 |                   4.3 |          82.7 |
| dupin-core session (`search`) |              2.5 |             3.0 |      1.5 |                   5.9 |          91.5 |
| dupin-mac (`apps.rs`)         |              2.2 |             2.7 |      1.2 |                   6.8 |          78.7 |
| docs only                     |              0.2 |             0.2 |      0.2 |                   0.2 |             — |

Each edit rebuilt only the edited crate and the workspace crates above it.
No third-party unit was rebuilt, and alternating `build`, `test` and `clippy`
rebuilt nothing, so no feature difference between those commands forces
rebuilds. Appending an item and editing a function body cost the same.

### Nix checks

Seconds per check, each built alone
([results](results/2026-11-23-nix-checks.json)):

| Edit               | lint-and-test | hawk | dupin (release app) |
| ------------------ | ------------: | ---: | ------------------: |
| dupin UI           |          36.2 | 74.5 |               106.4 |
| dupin-core session |          36.4 | 66.5 |               100.1 |
| dupin-mac          |          34.3 | 63.5 |               103.8 |
| docs only          |           0.1 |  0.1 |                 0.0 |
| `VERSION` bump     |          39.4 | 63.7 |               101.5 |

Docs-only edits build nothing. The dependency outputs were reused for every
edit, including the `VERSION` bump. Before this branch they carried the
release version in their name, so each release rebuilt both.

## Reading

Editing deeper in the graph costs a few seconds more, but no split of the
`dupin` crate can save more than the 1.8 seconds an edit there takes now
incrementally. Release builds take 79–92 seconds whichever crate changes:
fat LTO with one codegen unit recompiles the whole program after any edit,
so compiling fewer crates first barely shortens it. Nix builds the workspace
without incremental compilation, where the gap between the cheapest and the
dearest edit is under three seconds.

At the time of measurement the Hawk check rebuilt every dependency with its own
toolchain on each edit, which is why it took about twice as long as Clippy and
the tests together. With its dependencies cached like the others, Hawk took
12 seconds after the UI edit, and building all three checks at once, as CI's
`build` job does, took 107 seconds instead of 142.
