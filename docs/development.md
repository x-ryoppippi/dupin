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

| File                   | Holds                                                                                                                       |
| ---------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| `flake.nix`            | Inputs and the flake-parts modules below                                                                                    |
| `package.nix`          | The Dupin package (`callPackage` style), with `Dupin.app` around the binary and its icon drawn from `assets/logo/dupin.svg` |
| `default.nix`          | `package.nix` for non-flake use                                                                                             |
| `nix/packages.nix`     | `packages.dupin` and `packages.default`                                                                                     |
| `nix/home-manager.nix` | `homeManagerModules.default`: `programs.dupin`, see [Configuration](./configuration.md#nix)                                 |
| `nix/checks.nix`       | Clippy and the tests in one debug build                                                                                     |
| `nix/treefmt.nix`      | The formatters and linters behind `nix fmt` and `checks.treefmt`                                                            |
| `nix/dev-shell.nix`    | The development shell                                                                                                       |

The version lives in `VERSION` (see [releasing](./releasing.md)). `package.nix` holds the hash of the vendored Cargo dependencies; after `Cargo.lock` changes, refresh it with `nu scripts/update-cargo-hash.nu` (nix-update). The vendor derivation's name carries `Cargo.lock`'s hash, so a stale hash fails the next local build instead of only CI. On every pull request, the autofix.ci workflow runs the same script and pushes the refreshed hash.

## Formatting

`nix fmt` runs treefmt with nixfmt, deadnix and statix for Nix, rustfmt, oxfmt (configured in `.oxfmtrc.json`) for JSON, YAML, TOML and Markdown, nufmt for Nushell, typos (`typos.toml`), and actionlint and zizmor for the workflows. `nix flake check` fails on anything left unformatted.

## Headless renders

Debug builds take Rune's headless flags (`--snap`, `--tree`, `--script`, `--realtime`, ...; see Rune's `crates/rune/README.md`, "Agent tooling"). `scripts/snap.nu` renders every scenario in `scripts/headless/` to `target/screenshots/`:

```sh
cargo build -p dupin
nu scripts/snap.nu
```

The renders show real results from the machine, file paths included, so they stay out of the repository. Headless renders draw the panel's plain card where a window shows Liquid Glass.

## Peak memory

Dupin's physical footprint peaks at about 380–400 MB while it draws, then falls back to about 30 MB. Almost all of the peak is the Apple GPU driver's: the first render pass a process submits charges it about 185 MB, and the first blit (Rune's texture uploads) about 135 MB more, whatever the size of the work. The driver frees it within a second of the GPU going idle and charges it again on the next frame. A bare Metal blit and render pass from `osascript`, with no Rune, shows both charges and peaks at 327 MB; Rune's own `examples/hello` peaks at 373 MB and Dupin at 377 MB (headless `--snap` on an M5 Max). Launchers drawn with AppKit don't pay this, because WindowServer composites their layers out of process.

Dupin can't avoid this without drawing outside Metal. `scripts/footprint.nu` renders each headless scenario and fails if one's whole peak footprint, the driver's charge included, goes over a budget (450 MB by default). The driver's charge scales with the GPU, so the budget only holds for one Mac and build configuration; pass `--budget` on others:

```sh
cargo build -p dupin
nu scripts/footprint.nu
```

## Timing a summon

`DUPIN_TRACE=1` prints when the launch hotkey is registered and, for each summon, when the hotkey handler ran and when the next frame was drawn, with wall-clock times to line up with an outside measurement of when the window reached the screen.

Scripts in this repository are written in Nushell.

## Logo

`assets/logo/dupin.svg` is the app icon and `assets/logo/dupin-mark.svg` a monochrome `currentColor` mark, for template images such as a menu-bar item. They are the only tracked logo sources: the Nix build draws every icon size from `dupin.svg` with resvg and packs them with icnsutil, so no generated PNG or icns is committed.

## Website

`website/` holds <https://dupin.app>, with its own pnpm project. Pages are Svelte components in `website/src/pages/` that [ox-content](https://github.com/ubugeeei-prod/ox-content)'s custom host (`website/src/host/index.ts`) renders to static HTML; the browser then hydrates the same component from the props written into the page, which runs the panel demo, the copy buttons and the download channel switch. The development shell provides Node.js and the pnpm version that `website/package.json` pins (`nix flake check` fails when they differ).

```sh
just website::install # dependencies from the lockfile
just website::dev     # local server, rendering the real pages
just website::preview # build, then serve the result with the redirects
just website::lint    # oxlint, type-aware and type-checked through tsrs
just website::deploy  # cf build, then upload the result as Cloudflare static assets
```

Building and deploying go through `cf`, with the Cloudflare Vite plugin producing the Build Output. Workers Builds runs `pnpm run deploy` on `main` (the `dupin` Worker on `dupin.app`) and `cf previews deploy` on other branches, whenever `website/` or `assets/logo/` changes. `dupin.app/gh` redirects to the repository; the redirects are declared in `website/vite.config.ts`.

Icons are `icon-[set--name]` classes that ox-content turns into one self-hosted stylesheet from the `@iconify-json/*` collections installed in `website/`; add the collection before using a new set. For agents and crawlers the site also serves `/llms.txt`, a Markdown twin of each page (`/index.md`, `/download.md`), `sitemap.xml` and `robots.txt`; the Markdown is built in `website/src/markdown.ts` from the same data as the pages, and `website/public/_headers` sets their content types.

The shared page head supplies canonical URLs, social cards and application JSON-LD.
The 1200 × 630 social image is generated from `assets/social/og.svg`, which reuses
the app logo. After editing the source, regenerate `website/public/og.png`:

```sh
nix shell --inputs-from . nixpkgs#resvg --command resvg --skip-system-fonts \
  --use-fonts-dir "$(nix build --inputs-from . --no-link --print-out-paths nixpkgs#inter)/share/fonts" \
  assets/social/og.svg website/public/og.png
```
