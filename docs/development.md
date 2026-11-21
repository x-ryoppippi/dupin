# Development

Dupin targets Apple silicon macOS only. It builds with Nix; the Rust toolchain is nixpkgs' own, which matches the version Rune's `rust-toolchain.toml` pins.

## Build and run

```sh
nix build .#dupin                  # result/Applications/Dupin.app
open result/Applications/Dupin.app
nix flake check                    # treefmt, clippy, tests and the app bundle
nix fmt                            # format everything (see Formatting)
```

`direnv allow` (or `nix develop`) gives a shell with cargo, clippy, rustfmt, rust-analyzer, nushell and the formatters:

```sh
cargo run -p dupin
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p dupin-mac --example search -- in invoice # a provider from the terminal: apps, files or in
```

## Nix layout

| File                | Holds                                                                       |
| ------------------- | --------------------------------------------------------------------------- |
| `flake.nix`         | Inputs and the flake-parts modules below                                    |
| `package.nix`       | The Dupin package (`callPackage` style), with `Dupin.app` around the binary |
| `default.nix`       | `package.nix` for non-flake use                                             |
| `nix/packages.nix`  | `packages.dupin` and `packages.default`                                     |
| `nix/checks.nix`    | Clippy and the tests in one debug build                                     |
| `nix/treefmt.nix`   | The formatters and linters behind `nix fmt` and `checks.treefmt`            |
| `nix/dev-shell.nix` | The development shell                                                       |

The version lives in `VERSION` (see [releasing](./releasing.md)). `package.nix` holds the hash of the vendored Cargo dependencies; after `Cargo.lock` changes, refresh it with `nu scripts/update-cargo-hash.nu` (nix-update). The vendor derivation's name carries `Cargo.lock`'s hash, so a stale hash fails the next local build instead of only CI.

## Formatting

`nix fmt` runs treefmt with nixfmt, deadnix and statix for Nix, rustfmt, oxfmt (configured in `.oxfmtrc.json`) for JSON, YAML, TOML and Markdown, nufmt for Nushell, typos (`typos.toml`), and actionlint and zizmor for the workflows. `nix flake check` fails on anything left unformatted.

## Headless renders

Debug builds take Rune's headless flags (`--snap`, `--tree`, `--script`, `--realtime`, ...; see Rune's `crates/rune/README.md`, "Agent tooling"). `scripts/snap.nu` renders every scenario in `scripts/headless/` to `target/screenshots/`:

```sh
cargo build -p dupin
nu scripts/snap.nu
```

The renders show real results from the machine, file paths included, so they stay out of the repository. Headless renders draw the panel's plain card where a window shows Liquid Glass.

## Timing a summon

`DUPIN_TRACE=1` prints, for each summon, when the hotkey handler ran and when the next frame was drawn, with wall-clock times to line up with an outside measurement of when the window reached the screen.

Scripts in this repository are written in Nushell.
