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

For rebuild times and the sccache decision, see [Build caching](#build-caching).

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

The version lives in `VERSION` (see [releasing](./releasing.md)). `nix/cargo-vendor.nix` vendors the Cargo dependencies from `Cargo.lock`. crates.io packages come by their lock checksums, so a lock change needs no hash update. Git packages (only Rune) come by a hash per locked rev in that file, so a fully cached build evaluates without cloning them; evaluation fails with how to get the new hash when a locked rev has none. Every Nix build of the workspace uses that one vendor tree.

Flake builds keep compiled dependencies in four outputs, one each for the
release binary, Clippy and the tests, Hawk, whose toolchain differs, and the
latency binary. A source edit reuses all four; only a manifest or `Cargo.lock`
change rebuilds them. Hawk's dependency build runs with a fixed configuration,
so editing `hawk.toml` or `scripts/hawk.nu` reuses it too. Every
dependency-only output is named with version 0.0.0 rather than `VERSION`, so a
release bump reuses them as well; `checks.<system>.dependency-cache-keys` fails
if one would not. The two check builds leave out debug info, which a failing
test does not need to report where it panicked.
The [Nix cache experiment](../experiments/nix-cargo-cache/README.md) compares
source-edit builds against the original builder. `dupin.baseline` exposes that
builder. The non-flake `default.nix` build uses it too, with crane at the rev
`flake.lock` pins.

### Rune updates

Rune has no releases, so `Cargo.toml` pins every Rune crate, and every crate Rune's own workspace patches, at one commit of Rune's default branch, `dev`. An update flows through three steps:

1. Renovate proposes the newest `dev` commit once a week (Mondays, 00:00–06:00 UTC) as one pull request that rewrites every rev. A Git commit has no release timestamp, so the three-day release age can't apply; instead the pull request never automerges, and a person merges it.
2. On that pull request, the `autofix.ci` workflow runs `scripts/update-rune.nu`. It relocks Rune's packages in `Cargo.lock`, along with any crate `[patch.crates-io]` takes from Rune that the lock still takes from elsewhere, and recomputes Rune's source hash in `nix/cargo-vendor.nix`, and autofix.ci commits the result. If a patch still doesn't apply after that, because Dupin's own requirement rules out the version Rune patches, the script fails rather than leave the crate unpatched. When the lock and the hash already match the pinned rev, the script changes nothing and reaches no network, so the workflow runs on every pull request that touches `Cargo.toml`, `Cargo.lock` or `nix/cargo-vendor.nix`.
3. CI builds and tests the result like any other change.

Before merging, read Rune's changelog (`changelog/unreleased/` in its repository) between the two revs. Renovate can't add a crate Rune's workspace newly patches, and an item in [Working around Rune](./rune-workarounds.md) may no longer be needed. To bump Rune by hand, change every rev and run the script; the dev shell doesn't evaluate until the hash matches, so take its tools from nixpkgs:

```sh
nix shell --inputs-from . nixpkgs#nushell nixpkgs#cargo --command nu scripts/update-rune.nu
```

## Build caching

The [crate boundary experiment](../experiments/crate-boundaries/README.md)
times representative edits. On a 6-CPU Testbox, an incremental `cargo build -p
dupin` after editing a UI file, `dupin-core`'s session code or `dupin-mac` takes
1.8–2.5 seconds, `cargo test --workspace --no-run` 1.8–3.0 and `cargo clippy`
0.8–1.5. A docs-only edit rebuilds nothing. Alternating `build`, `test` and
`clippy` rebuilds nothing either, so no feature difference between them
forces rebuilds. A release build takes 79–92 seconds whichever crate changed,
because fat LTO recompiles the whole program. These numbers justify no
crate split: editing the `dupin` crate already costs the least. New crates
should stay leaves that depend on `serde` and `libc`, as the extension crates
do, rather than on `dupin-core`, `dupin-mac` or Rune.

CI's `build` job builds Clippy and the tests, Hawk and the release app in one
`nix build`. Across 59 recent successful runs, that step took 143–191 seconds
(median 155) for a Rust edit, about 340 seconds when `Cargo.lock` or a
manifest changed, and 16–18 seconds when everything came from the cache. 17
runs could not download a path that the Hestia manifest still listed, because
its pack had been evicted from the cache; nine of them rebuilt the app for a
change that touched no Rust. Each `Cargo.lock` change stores about 1 GB of
compiled dependencies. On the Testbox, the same step after a UI edit takes 107
seconds instead of 142 now that Hawk reuses its dependencies, and the three
dependency outputs that step builds together hold 1.03 GB (debug 353 MB,
release 343 MB, Hawk 338 MB), where the debug output alone held 662 MB with
debug info.

sccache is not configured for local builds or CI. Cargo builds workspace
crates incrementally in the dev profile, which sccache cannot cache, and those
crates are all that an edit rebuilds. It can only replay third-party crates in
a fresh target directory. In the
[Cargo cache experiment](../experiments/cargo-cache/README.md) on the Testbox,
a warm cache built a fresh target at a new path, as a new worktree has, in 46.5
seconds against 48.8–49.5 without it, and a cold cache cost 1.2 seconds more.
It only pays when a removed target is rebuilt at the same path: 18.1 seconds
against 47.5. CI compiles through Nix, which caches whole dependency outputs
instead. The development shell does not include sccache. A developer who often
deletes and rebuilds the same target directory can run it from nixpkgs without
installing it:

```sh
nix build --inputs-from . nixpkgs#sccache --out-link target/sccache
RUSTC_WRAPPER="$PWD/target/sccache/bin/sccache" cargo build
```

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

Dupin's physical footprint peaks at about 360–395 MB while it draws, then falls back to about 30 MB. Almost all of the peak is the Apple GPU driver's. It charges a process once per kind of encoder, whatever the size of the work: about 185 MB for a render pass and about 135 MB for a blit or compute encoder. It frees the charge within a second of the GPU going idle and charges it again on the next frame. Launchers drawn with AppKit don't pay this, because WindowServer composites their layers out of process.

Dupin encodes no blits itself. Rune's Metal backend does. Since Rune fcef33a2 ([anomalyco/rune#1764](https://github.com/anomalyco/rune/pull/1764)), textures the CPU fills, such as the glyph and image atlases, live in shared storage and are zeroed and written on the CPU, which took about 20 MB off every scenario's peak (`empty`: 383 MB to 361 MB). The blit's charge remains, because Rune still blits in other places. A buffer write that Rune can't make in place, because the GPU still reads that buffer, is staged and copied with a blit at the next flush, which happens on nearly every frame. Headless runs also zero their offscreen render target and read `--snap` frames back with blits. With every blit done on the CPU in a patched copy of Rune 86b903de, Rune's `examples/hello` peaks at 218 MB instead of 371 MB on an M5 Max, which leaves only the render pass's charge. What remains is tracked in [anomalyco/rune#1753](https://github.com/anomalyco/rune/issues/1753). The render pass's charge stays as long as Dupin draws with Metal.

`scripts/footprint.nu` renders each headless scenario and fails if one's whole peak footprint, the driver's charge included, goes over `--budget` (405 MB by default), or if it rises more than `--share` (45 MB) above the `empty` scenario's. Every scenario pays the same driver charge, so the share measures Dupin's own memory rather than the GPU's; the whole-peak budget only holds for one Mac and build configuration, so pass `--budget` on others:

```sh
cargo build -p dupin
nu scripts/footprint.nu
```

## Timing a summon

`DUPIN_TRACE=1` prints when the launch hotkey is registered and, for each summon, when the hotkey handler ran and when the next frame was drawn, with wall-clock times to line up with an outside measurement of when the window reached the screen.

## Keypress latency

`scripts/latency.nu` measures how long Dupin takes from a typed key to the first frame that shows every provider's results for the new query. It drives a headless `--realtime` run with Rune's scripted keys, which reach the query field and the session as a window's keys do, against the same fixtures as the headless renders (calendar, dictionary, clipboard), so it needs no permissions. `DUPIN_LATENCY=<file>` makes a headless run write one JSON line per edit of the query; a window never writes it.

```sh
cargo build --profile latency -p dupin --features tooling   # target/latency/dupin
nu scripts/latency.nu run                                    # target/latency/results.json
nu scripts/latency.nu run --only calendar --runs 1           # a quick look at one feature
nu scripts/latency.nu compare main.json pr.json              # fails on a p95 regression
nu scripts/latency.nu parity target/latency/results.json     # extensions against native
```

`nix build .#dupin-latency` builds the same binary. The `latency` profile is the release profile with thin LTO and Rune's headless tooling, so numbers are only comparable between builds of that profile on one machine.

Each scenario in `scripts/latency/scenarios.toml` names a feature, an unmeasured prefix (`cal `) and the characters measured one keypress at a time (`rev`). Each implementation of a feature names the provider that answers it; the run fails when a measured keypress does not search that provider, or the whole of the measured characters shows no rows from it, so a scenario that never reaches its feature cannot pass as a measurement of it. A run types the scenario `--warmup` times unmeasured, then until `--samples` keypresses (210 by default) are measured, `--gap` apart. Each scenario runs `--runs` times (three by default), each in a fresh process, and every reported figure is the median of the runs' own, so one run disturbed by the rest of a shared machine does not move the result. A keypress typed over before its results were drawn has no latency; the run fails when more than 1% are, as the gap is then too short.

For each scenario the results give p50, p95 and p99 of the total, and p50 and p95 of each stage, which add up to it:

- `dispatch_ms`: from the edit to the searches it starts being handed to their providers' workers.
- `provider_ms`: from there to the last provider's final delivery leaving its worker. `providers` in the raw lines has each one's time.
- `merge_ms`: from there to the session holding the finished, ranked results. It includes waiting for the UI thread, which may be drawing a frame.
- `layout_ms`: from there to the next frame being built, laid out and encoded. Headless frames start at most every `1 / --fps` seconds, so this includes up to a millisecond of waiting at the default 1000 fps.

`compare` holds a candidate result set against a baseline, scenario by scenario, and fails when the candidate's p95 exceeds the baseline's by more than `--tolerance` percent (10 by default) or `--floor` (1 ms), whichever is larger. `--report` prints the comparison without failing. `--baseline-implementation` and `--candidate-implementation` compare two implementations across result sets.

A scenario's id names what a person types, not the code that answers it. When a bundled extension replaces a native feature, declare it beside the native implementation under the same feature, with the config that enables it and its extensions directory:

```toml
[features.calendar.extension]
config = "scripts/latency/calendar-extension.toml"
provider = "<the extension's command id>"
extensions = "scripts/latency/calendar-extension.extensions"
```

Every scenario of that feature then also runs with the extension, under the same id, and `parity` fails when its p95 exceeds native's beyond the same tolerance. That is the gate for deleting the native implementation. A port can instead `extend` the native implementation and name only what it changes; `settings` is merged over the config it starts from:

```toml
[features.snippets.extension]
extends = "native"
settings = { extensions = true, snippets = { enabled = false } }
provider = "extension.snippets"
extensions = "scripts/latency/snippets.extensions"
```

A bundled extension that adds a feature Dupin never had natively has no native implementation to keep parity with. It declares `budget_ms` instead, and `parity` fails when its p95 exceeds it; Emoji & Symbols is held to 5 ms:

```toml
[features.emoji.extension]
config = "scripts/headless/config/dupin/config.toml"
settings = { extensions = true }
provider = "extension.emoji"
extensions = "extensions"
budget_ms = 5.0
```

The latency gate has a behavioral twin in the `dupin` crate's tests: `extensions::parity::Parity` holds the native provider and the port's to the same rows, sections, details and actions for a list of queries. `scripts/latency/catalogue.extensions` holds a 5,000-item `items()` package, measured under its keyword (`catalogue`) and in the main search (`search-catalogue`); neither replaces a built-in, so neither has a native implementation to hold to parity.

The Latency workflow runs on pull requests that touch `crates/`, `extensions/` or the benchmark, and on `main`. It uploads `results.json` as the `latency` artifact; a pull request's summary compares it with the latest from `main` but does not fail on it, since a shared runner's noise is larger than many real changes. The parity check does fail the job. To see whether a reported regression is real, re-run the job, or run both commits on one quiet machine and compare those.

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
