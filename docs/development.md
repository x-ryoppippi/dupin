# Development

Dupin targets Apple silicon macOS only. It builds with Nix; the Rust toolchain is nixpkgs' own, which matches the version Rune's `rust-toolchain.toml` pins.

## Build and run

```sh
nix build .#dupin                  # result/Applications/Dupin.app
open result/Applications/Dupin.app
nix flake check                    # treefmt, clippy, tests and the app bundle
nix fmt                            # format everything (see Formatting)
```

In the development shell, `just` runs every build; `just` alone lists the recipes:

```sh
just build-all   # the app bundle, the debug binary and the website
just build       # nix build .#dupin
just build-debug # cargo build -p dupin
just run         # build the bundle, then open it
just package     # a signed dist/Dupin.zip; see releasing
just snap        # headless renders (see Headless renders)
just footprint   # the peak-footprint budget (see Peak memory)
```

`direnv allow` (or `nix develop`) gives a shell with cargo, clippy, rustfmt, rust-analyzer, nushell and the formatters:

```sh
cargo run -p dupin
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p dupin-mac --example search -- in invoice # a provider from the terminal: apps, files or in
```

For isolated local Cargo cache measurements, see the
[Cargo cache experiment](../experiments/cargo-cache/README.md). It compares
sccache across fresh and reused target directories without changing the
development shell or Nix package cache.

## Remote development with Testbox

The development shell includes Blacksmith CLI 0.5.3. The manually dispatched
`testbox-dev.yml` workflow compiles native test dependencies on a disposable Apple
silicon runner, then keeps it available for commands. It follows ccusage's
Testbox setup and uses the same macOS runner and Nix setup as CI.

After this workflow is available on the repository's default branch, start a box
from the root of the worktree you want to test:

```sh
blacksmith auth status
blacksmith --org <account> testbox warmup .github/workflows/testbox-dev.yml \
  --ref <pushed-branch> --job dev --idle-timeout 15
blacksmith --org <account> testbox run --id <id> \
  'nix develop --command cargo clippy --workspace --all-targets -- -D warnings'
blacksmith --org <account> testbox run --id <id> \
  'nix develop --command cargo test --workspace'
blacksmith --org <account> testbox stop --id <id>
```

Use the Blacksmith account connected to this repository. Each agent or worktree
gets its own box and ID. Run commands from the worktree root: the CLI mirrors
that directory with `rsync --delete`, including tracked and untracked files
that Git does not ignore. Ignored build directories stay on the runner. Check
which files will be exported before the first run; a Git-ignored file is not
evidence that every other local file is safe to send.

Use Cargo directly for iteration. Synced new files are present on disk but are
not added to the remote Git index, so a Git flake build can omit them. Cargo
reads them directly and reuses the box's ignored `target/` between runs. Push
new Nix inputs before warming a ref that needs them.

The box holds its own configuration and build outputs. Native E2E additionally
needs agent-desktop and the disposable runner's accessibility, automation and
screen recording grants; this workflow does not configure or verify those.

Stop a box when finished; the idle timeout is a fallback and the workflow has a
four-hour limit. CLI builds and workflow checks do not prove that remote
authentication, warmup or native E2E succeeded.

## Dependency builds

The version lives in `VERSION` (see [releasing](./releasing.md)). `package.nix` holds the hash of the vendored Cargo dependencies; after `Cargo.lock` changes, refresh it with `nu scripts/update-cargo-hash.nu` (nix-update). The vendor derivation's name carries `Cargo.lock`'s hash, so a stale hash fails the next local build instead of only CI. On every pull request, the autofix.ci workflow runs the same script and pushes the refreshed hash.

Flake builds share compiled dependencies separately for release and debug.
The [Nix cache experiment](../experiments/nix-cargo-cache/README.md) compares
source-edit builds against the original builder. `dupin.baseline` exposes that
builder and its vendor-hash definition; the hash update script evaluates it so
updates still target `package.nix`. The non-flake `default.nix` build uses it too.

## Formatting

`nix fmt` runs treefmt with nixfmt, deadnix and statix for Nix, rustfmt, oxfmt (configured in `.oxfmtrc.json`) for JSON, YAML, TOML and Markdown, nufmt for Nushell, typos (`typos.toml`), and actionlint and zizmor for the workflows. `nix flake check` fails on anything left unformatted.

## Headless renders

Debug builds take Rune's headless flags (`--snap`, `--tree`, `--script`, `--realtime`, ...; see Rune's `crates/rune/README.md`, "Agent tooling"). `scripts/snap.nu` renders every scenario in `scripts/headless/` to `target/screenshots/`:

```sh
cargo build -p dupin
nu scripts/snap.nu
nu scripts/snap.nu --only calendar --theme dark # one scenario, in one appearance
```

`--theme light` or `--theme dark` fixes the appearance and adds it to each file name; without it the renders follow the Mac's. The renders show real results from the machine, file paths included, so they stay out of the repository. Calendar and dictionary scenarios use fixed fixtures instead. Headless renders draw the panel's plain card where a window shows Liquid Glass.

## Native E2E evidence

CI collects screenshots and any MP4, MOV or WebM recordings in `e2e-media`, with
logs and detailed replies in `e2e`. The reporting workflow uses the same body
for PR comments and commit comments: images render inline and recordings use
GitHub's video player. Existing `e2e-screenshots` runs remain readable.

Automatic attachment uploads need the optional repository secret
`E2E_ASSET_TOKEN`, containing an accepted GitHub OAuth or personal access token
with write access to the repository. The ordinary `GITHUB_TOKEN` can post the
comment but currently cannot upload user attachments. No upload credential is
created automatically. Without it, or if the upload fails, the comment retains
the media download and logs. Inline uploads are limited to runs whose source
repository is this repository; fork PRs keep the download link rather than
publishing contributor-controlled files under the upload account.

The privileged reporting job checks out only the default branch and never runs
PR code. Its uploader accepts top-level regular media files with safe names and
matching signatures, up to 50 files, 10 MB per image or 100 MB per video; GitHub
may impose a lower account limit. Uploaded URLs must belong to GitHub's
attachment service. Optional setup, download and upload operations have bounded
timeouts so they leave time to post the result and fallback links. The raw
upload protocol follows GitHub CLI 2.104.0 and is
not a documented REST contract, so compatibility failures use the same fallback.

For local inspection, the uploader defaults to a JSON manifest without network
access. Explicit upload reads `GH_TOKEN` from the caller's environment:

```sh
nu scripts/e2e-assets.nu x-ryoppippi/dupin target/e2e
nu scripts/e2e-assets.nu x-ryoppippi/dupin target/e2e --upload
nu scripts/tests/e2e-assets.nu
```

The native CI suite also records the first 45 seconds of its real interaction:
summoning the panel, typing the search term character by character, showing the
results and opening Calculator. `--record` uses the runner's existing Screen
Recording grant; it does not request or change permissions. It waits for the
FFmpeg AVFoundation capture to encode its first frame before sending keys. It
captures the screen without audio, writes a bounded 45-second H.264 MP4, and
checks its duration and frame count before attaching it. A process deadline
bounds capture failures; logs and rejected footage remain in the full artifact,
including after a scenario failure. Local runs record only when explicitly
given `--record`; use a disposable machine. Native capture and encoding must be
verified by CI.

## Peak memory

Dupin's physical footprint peaks at about 375–400 MB while it draws, then falls back to about 30 MB. Almost all of the peak is the Apple GPU driver's. It charges a process once per kind of encoder, whatever the size of the work: about 185 MB for a render pass and about 135 MB for a blit or compute encoder. It frees the charge within a second of the GPU going idle and charges it again on the next frame. Launchers drawn with AppKit don't pay this, because WindowServer composites their layers out of process.

Dupin encodes no blits itself. Rune's Metal backend does: it zeroes every new texture with a blit, uploads glyph and image rows with blits, and reads `--snap` frames back with one. Rune's `examples/hello` peaks at 371 MB on an M5 Max, and at 218 MB with those done on the CPU through shared textures, which leaves only the render pass's charge. That fix belongs in Rune: [anomalyco/rune#1753](https://github.com/anomalyco/rune/issues/1753), and [#1245](https://github.com/anomalyco/rune/issues/1245) for uploads while drawing. The render pass's charge stays as long as Dupin draws with Metal.

`scripts/footprint.nu` renders each headless scenario and fails if one's whole peak footprint, the driver's charge included, goes over `--budget` (410 MB by default), or if it rises more than `--share` (35 MB) above the `empty` scenario's. Every scenario pays the same driver charge, so the share measures Dupin's own memory rather than the GPU's; the whole-peak budget only holds for one Mac and build configuration, so pass `--budget` on others:

```sh
cargo build -p dupin
nu scripts/footprint.nu
```

## Timing a summon

`DUPIN_TRACE=1` prints when the launch hotkey is registered and, for each summon, when the hotkey handler ran and when the next frame was drawn, with wall-clock times to line up with an outside measurement of when the window reached the screen.

Scripts in this repository are written in Nushell.

## Rust visibility

`just hawk` checks workspace API visibility and fails on warnings. The same
command runs in `nix build .#checks.aarch64-darwin.hawk`, which CI includes
alongside its existing checks. `hawk.toml` identifies the distributed launcher
as the production binary.

Hawk 0.1.18 uses a private Rust 1.100.0 analysis toolchain. Its compiler-internal
API requires that exact toolchain; `cargo-hawk` selects it without changing the
compiler used by ordinary Cargo commands or the packaged launcher. Update the
Hawk archive and its analysis compiler together in `nix/cargo-hawk.nix`.

`scripts/hawk.nu` excludes the plugin protocol, runtime and policy libraries
from closed-workspace visibility decisions. Those libraries define plugin and
host contracts, including consumers in separate experiment workspaces, so this
workspace's callers cannot establish their complete public API requirements.

## Logo

`assets/logo/dupin.svg` is the app icon and `assets/logo/dupin-mark.svg` a monochrome `currentColor` mark, for template images such as a menu-bar item. They are the only tracked logo sources: the Nix build draws every icon size from `dupin.svg` with resvg and packs them with icnsutil, so no generated PNG or icns is committed.

## Website

`website/` holds <https://dupin.app>, with its own pnpm project. Pages are Svelte components in `website/src/pages/` that [ox-content](https://github.com/ubugeeei-prod/ox-content)'s custom host (`website/src/host/index.ts`) renders to static HTML; the browser then hydrates the same component from the props written into the page, which runs the panel demo, the copy buttons and the download channel switch. The development shell provides Node.js and the pnpm version that `website/package.json` pins (`nix flake check` fails when they differ).

```sh
just website::install # dependencies from the lockfile
just website::dev     # local server, rendering the real pages
just website::preview # build, then serve the result with the redirects
just website::lint    # oxlint, type-aware and type-checked through tsrs
just website::deploy  # build and deploy the Worker and its static assets
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
