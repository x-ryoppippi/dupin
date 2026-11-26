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
just docs-images # the images in docs/images (see Documentation images)
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
2. On that pull request, the `autofix.ci` workflow runs `scripts/update-rune.nu`. It relocks Rune's packages in `Cargo.lock`, along with any crate `[patch.crates-io]` takes from Rune that the lock still takes from elsewhere, and recomputes Rune's source hash in `nix/cargo-vendor.nix`, and autofix.ci commits the result. If a patch still doesn't apply after that, because Dupin's own requirement rules out the version Rune patches, the script fails rather than leave the crate unpatched. When the lock and the hash already match the pinned rev, the script changes nothing and reaches no network, so the workflow runs it on every pull request.
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

`nix fmt` runs treefmt with nixfmt, deadnix and statix for Nix, rustfmt, oxfmt (configured in `.oxfmtrc.json`) for JSON, YAML, TOML and Markdown, nufmt for Nushell, typos (`typos.toml`; it only reports, since a typo's fix needs a person), and actionlint and zizmor for the workflows. `nix flake check` fails on anything left unformatted.

On every pull request, the `autofix.ci` workflow runs `nix fmt` after the [Rune step](#rune-updates), and autofix.ci commits both fixes in one commit, so unformatted files come back formatted instead of failing CI. The flake targets only aarch64-darwin, but it exports the same treefmt configuration as `formatter.x86_64-linux`, so the workflow runs on a cheap Linux runner. The `treefmt` check in CI still fails where autofix.ci can't commit, such as a pull request from a fork that hasn't installed the app, and on what the formatters report but can't fix, such as a typo, an actionlint error or a zizmor finding. The workflow also runs on every push to `main`, without formatting or committing anything, because Blacksmith saves the job's Nix store only from `main`; without it, every pull request would download the formatter, Nushell and cargo again.

## Headless renders

Every build of Dupin, the released app included, takes Rune's headless flags (`--snap`, `--tree`, `--script`, `--realtime`, ...; see Rune's `crates/rune/README.md`, "Agent tooling"). `scripts/snap.nu` renders every scenario in `scripts/headless/` to `target/screenshots/`:

```sh
cargo build -p dupin
nu scripts/snap.nu
nu scripts/snap.nu --only calendar --theme dark # one scenario, in one appearance
```

`--theme light` or `--theme dark` fixes the appearance and adds it to each file name; without it the renders follow the Mac's. The renders show real results from the machine, file paths included, so they stay out of the repository. Calendar and dictionary scenarios use fixed fixtures instead, and clipboard scenarios the history in `scripts/fixtures/clipboard.json`, or a scenario's own in `<name>.clipboard/history.json`, whose `image` entries name PNG files beside it. A scenario with a `<name>.snapshots/snapshots.json`, a list of snapshots, starts with them saved in scratch storage. A scenario with a `<name>.home/` folder runs with a copy of it as its home: apps in its `Applications` folder are found like any other, and its `Library/Application Support/Dupin/history.json` is the usage history, where `$HOME` in an app's path stands for the copy's path. A scenario's own snapshots, clipboard history, extensions, grants and home are read only beside its own `<name>.toml`. Headless renders draw the panel's plain card where a window shows Liquid Glass.

## Documentation site

dupin.app/docs is this folder, rendered when the website builds (`just website::build`): `website/src/docs/pages.ts` lists the pages it publishes, by section, and ox-content renders each one and indexes `docs/` for the site's search. A link to another published page goes to its page on the site, and a link to anything else, such as this page, a design document or `extensions/`, goes to GitHub. Add a new user-facing page to `pages.ts`; pages for people working on Dupin stay on GitHub only.

## Documentation images

The images in `docs/images` are rendered, never taken by hand. `scripts/docs-images/images.toml` lists each one; `just docs-images` (`nu scripts/docs-images.nu` after `cargo build -p dupin`) renders every one in light and dark to `docs/images/<page>/<name>-<theme>.png`, compresses them losslessly with oxipng and deletes the images the list no longer has. `--only <text>` renders just the images whose `<page>/<name>` contains it. Re-run it after a change to anything an image shows, and commit the result.

Every image comes from fixtures, so a render on any Mac shows nothing of that Mac's: a bundled or example extension's own fixture through `dupin ext snap`, or a headless scenario run in a scratch home that holds the fixture documents and usage history the list names, with the fixture clipboard history and snapshots, and with `DUPIN_HEADLESS_APPS=/System/Applications`. A headless run takes that `PATH`-style list in place of the app folders, so it lists only the stock apps; `dupin ext snap` sets it empty. Spotlight indexes nothing in a scratch home, and on some Macs not even a fresh disk image, so no image shows a Spotlight search: the file images start from Recent items. Settings pages are drawn from the `prefs` window (`--window prefs`).

A page embeds an image as a `<picture>` whose `prefers-color-scheme: dark` source is the dark image, so GitHub shows the one matching the reader's theme:

```html
<picture>
	<source media="(prefers-color-scheme: dark)" srcset="../images/calendar/upcoming-dark.png" />
	<img alt="What the image shows" src="../images/calendar/upcoming-light.png" width="680" />
</picture>
```

Pixels differ from Mac to Mac (fonts, app icons and the GPU), so CI doesn't render the images again. `checks.docs-images` runs `nu scripts/docs-images.nu check` instead: every listed image and clip exists in both themes, `docs/images` holds nothing else, some page embeds each one and every image a page embeds exists.

### Clips

The same list's `[[clips]]` are short recordings: each plays its `steps` (typed text, keys and waits) into the panel in a headless `--realtime` run, in the same scratch home as the images, and saves `docs/images/<page>/<name>-<theme>.webp` in light and dark. `just docs-clips` (`nu scripts/docs-images.nu --kind clips`) renders only the clips; `just docs-images` renders both. Rune writes the frames at 15 fps and 1.5× scale, each clip is cut to the lowest edge the panel reaches in any frame, and a caption below the panel names each key pressed (`⇥ Tab`, `⌘K`) or the text typed so far (`type: cal`). ImageMagick draws the captions in Inter, the same on every Mac, which the dev shell provides as `DUPIN_CAPTION_FONT`, and `img2webp` encodes the frames as a lossy animated WebP, in which a frame that repeats the last costs nothing. A clip is embedded like an image, as a `<picture>` of its two WebP files:

```html
<picture>
	<source media="(prefers-color-scheme: dark)" srcset="images/tutorial/apps-dark.webp" />
	<img alt="What the clip shows" src="images/tutorial/apps-light.webp" width="680" />
</picture>
```

Animated WebP, not video, because the same Markdown is read on GitHub, which shows a `<video>` only for files uploaded to an issue or pull request, never one in the repository; GitHub shows an animated WebP as an image, and so does every current browser, without a player or autoplay rules. A clip must never type into a Spotlight search (Space on an empty query, or `in`), since Spotlight answers from the rendering Mac's own index; the files clips start from Recent items and folder browsing instead, which read only the scratch home.

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

Dupin's physical footprint peaks at about 225–240 MB while it draws, then falls back to about 30 MB. Most of the peak is the Apple GPU driver's. It charges a process once per kind of encoder, whatever the size of the work: about 185 MB for a render pass and about 135 MB for a blit or compute encoder. It frees the charge within a second of the GPU going idle and charges it again on the next frame. Launchers drawn with AppKit don't pay this, because WindowServer composites their layers out of process.

Dupin's frames open only render passes, so they pay only the render pass's charge, which stays as long as Dupin draws with Metal. Dupin encodes no blits itself, and since Rune 253f4d6b ([anomalyco/rune#1821](https://github.com/anomalyco/rune/pull/1821), closing [anomalyco/rune#1753](https://github.com/anomalyco/rune/issues/1753)) Rune's Metal backend doesn't either: textures the CPU fills live in shared storage and are written in place ([anomalyco/rune#1764](https://github.com/anomalyco/rune/pull/1764)), and private render targets (offscreen targets, isolated layers, blurs, backdrop pyramids) are cleared by their first render pass instead of a zeroing blit. An lldb breakpoint on `-[AGXG17XFamilyCommandBuffer blitCommandEncoder]` no longer stops during a headless run without `--snap`. Before that the frames also paid the blit's charge, and the peak was about 360–395 MB. Only `--snap` still blits, to read the frame back, so a headless run with it peaks about 135 MB higher than one without.

`scripts/footprint.nu` runs each headless scenario without `--snap` and fails if one's whole peak footprint, the driver's charge included, goes over `--budget` (270 MB by default), or if it rises more than `--share` (30 MB) above the `empty` scenario's. On the M5 Max it was set on, `empty` peaks at about 225 MB and the busiest scenario about 17 MB above it, so a blit returning to the frame path, which costs every scenario about 135 MB, fails the budget. Every scenario pays the same driver charge, so the share measures Dupin's own memory rather than the GPU's; the whole-peak budget only holds for one Mac and build configuration, so pass `--budget` on others:

```sh
cargo build -p dupin
nu scripts/footprint.nu
```

## Timing a summon

`DUPIN_TRACE=1` prints when the launch hotkey is registered and, for each summon, when the hotkey handler ran and when the next frame was drawn, with wall-clock times to line up with an outside measurement of when the window reached the screen.

## Keypress latency

`scripts/latency.nu` measures how long Dupin takes from a typed key to the first frame that shows every provider's results for the new query. It drives a headless `--realtime` run with Rune's scripted keys, which reach the query field and the session as a window's keys do, against the same fixtures as the headless renders (calendar, dictionary, clipboard), so it needs no permissions. `DUPIN_LATENCY=<file>` makes a headless run write one JSON line per edit of the query; a window never writes it.

```sh
cargo build --profile latency -p dupin                       # target/latency/dupin
nu scripts/latency.nu run                                    # target/latency/results.json
nu scripts/latency.nu run --only calendar --runs 1           # a quick look at one feature
nu scripts/latency.nu compare main.json pr.json              # fails on a p95 regression
nu scripts/latency.nu parity target/latency/results.json     # extensions against native
```

`nix build .#dupin-latency` builds the same binary, inside a `Dupin.app` whose Resources hold the repository's `extensions/`, so it runs the bundled extensions as the released app does; outside one, a binary built away from the source tree finds none. The `latency` profile is the release profile with thin LTO and symbols kept, so numbers are only comparable between builds of that profile on one machine.

Each scenario in `scripts/latency/scenarios.toml` names a feature, an unmeasured prefix (`cal `) and the characters measured one keypress at a time (`rev`). Each implementation of a feature names the provider that answers it and, for an extension, the `package` behind it (`bundled:<name>` for one that ships with Dupin); the run fails when a measured keypress does not search that provider answered by that package, or by Dupin itself for an implementation without one, or the whole of the measured characters shows no rows from it, so a scenario that never reaches its feature cannot pass as a measurement of it. For a built-in command a bundled extension answers, only the rows from that package count as the provider's: the command shows a row of its own when its package is off, missing or failing, and that row is cheap enough to pass any budget. A run types the scenario `--warmup` times unmeasured, then until `--samples` keypresses (210 by default) are measured, `--gap` apart. Each scenario runs `--runs` times (three by default), each in a fresh process, and every reported figure is the median of the runs' own, so one run disturbed by the rest of a shared machine does not move the result. A keypress typed over before its results were drawn has no latency; the run fails when more than 1% are, as the gap is then too short.

For each scenario the results give p50, p95 and p99 of the total, and p50 and p95 of each stage, which add up to it:

- `dispatch_ms`: from the edit to the searches it starts being handed to their providers' workers.
- `provider_ms`: from there to the last provider's final delivery leaving its worker. `providers` in the raw lines has each one's time.
- `merge_ms`: from there to the session holding the finished, ranked results. It includes waiting for the UI thread, which may be drawing a frame.
- `layout_ms`: from there to the next frame being built, laid out and encoded. Headless frames start at most every `1 / --fps` seconds, so this includes up to a millisecond of waiting at the default 1000 fps.

`compare` holds a candidate result set against a baseline, scenario by scenario, and fails when the candidate's p95 exceeds the baseline's by more than `--tolerance` percent (10 by default) or `--floor` (1 ms), whichever is larger. `--report` prints the comparison without failing. `--baseline-implementation` and `--candidate-implementation` compare two implementations across result sets.

A scenario's id names what a person types, not the code that answers it. When a bundled extension replaces a native feature, declare it beside the native implementation under the same feature, with the config that enables it:

```toml
[features.clipboard.extension]
config = "scripts/latency/clipboard-extension.toml"
provider = "<the extension's command id>"
package = "<its package id>"
extensions = "scripts/latency/clipboard-extension.extensions"
```

Every scenario of that feature then also runs with the extension, under the same id, and `parity` fails when its p95 exceeds native's by more than 10% or 0.5 ms, whichever is larger. Its floor is half `compare`'s because native's p95 varies by only about a quarter of a millisecond between CI runs, and 1 ms would let a port of a 1.4 ms scenario be 70% slower. That is the gate for deleting the native implementation. Once it is deleted, the extension keeps the scenarios as the feature's only implementation, as Calculator's, Calendar search's, Clipboard history's, Dictionary's and Snippets' do. A port can instead `extend` the native implementation and name only what it changes; `settings` is merged over the config it starts from:

```toml
[features.notes.extension]
extends = "native"
settings = { extensions = true, notes = { enabled = false } }
provider = "extension.notes"
package = "notes"
extensions = "scripts/latency/notes.extensions"
```

A bundled extension that adds a feature Dupin never had natively has no native implementation to keep parity with. It declares `budget_ms` instead, and `parity` fails when its p95 exceeds it; Emoji & Symbols is held to 5 ms:

```toml
[features.emoji.extension]
config = "scripts/headless/config/dupin/config.toml"
provider = "extension.emoji"
package = "bundled:emoji"
budget_ms = 5.0
```

The latency gate has a behavioral twin in the `dupin` crate's tests: `extensions::parity::Parity` holds the native provider and the port's to the same rows, sections, details and actions for a list of queries. `scripts/latency/catalogue.extensions` holds a 5,000-item `items()` package, measured under its keyword (`catalogue`) and in the main search (`search-catalogue`); neither replaces a built-in, so neither has a native implementation to hold to parity, and both are held to a 5 ms budget.

### Native baselines

Once a port lands and its native implementation is deleted, CI has no native run to hold it to. `parity` then holds each scenario to the native results recorded in `scripts/latency/native-baseline.json`, which come from the last commit that had the native implementation, with the same tolerance. A scenario recorded there that the run doesn't measure fails the check, so a port can't go unmeasured by dropping or renaming its scenario. Likewise an implementation besides native with no native result in the run, no recorded baseline and no `budget_ms` fails it, so deleting a native implementation without recording its baseline can't take the port out of the gate.

Each recorded result also carries the `benchmark` fingerprint of what measured it: `scripts/latency/measure.nu` (its code, not its comments), the shared clipboard fixture, the scenario's `prefix` and `typed`, and the run's settings. Every `run` result carries the fingerprint of the benchmark it ran, and `parity` refuses a recorded baseline whose fingerprint differs from the port's, so changing how scenarios are measured can't leave a port held to a figure the new benchmark would not reproduce. Each implementation's own config is not part of it, since that is what parity compares.

A recorded figure comes from another run, and machines of one runner class differ in speed by more than the tolerance: the same native scenario's p95 has differed by a quarter between runs, which on its own decides whether a port passes. So each entry also records its `control`: the p95 of each native `search` scenario (`CONTROL` in `scripts/latency.nu`) in the same results, and the median of those. `parity` multiplies the recorded p95 by this run's median control p95 over the recorded one before applying the tolerance, and its table shows the recorded p95, that `scale` and the resulting `limit_ms`. The control is the median of every `search` scenario rather than one, since each has its own noise. An entry recorded without a control, or a run missing one of its control scenarios, fails `parity`, as does a control scenario measured by another benchmark than the recorded one. A pair held to its run's own native result is not scaled.

Record a feature's baseline in the pull request that deletes its native implementation, and re-record it when `parity` reports it stale, or when the runner class or what a fair comparison means changes:

```sh
# Measure the last commit with the native implementation; each run is the median of three.
# `ref` may be a short SHA, a branch or a tag; each dispatch runs to completion on its own.
gh workflow run latency.yml --ref <your branch> -f ref=<last native commit>
gh run download <run id> --name latency --dir target/latency/native-1
# Repeat for more runs if you like, then record the median of each figure:
nu scripts/latency.nu baseline dictionary target/latency/native-*/results.json
```

A Blacksmith Testbox measures the same way: warm a box up, check out the commit there, build `.#dupin-latency` and run `nu scripts/latency.nu run --implementation native` once per result. The control makes either machine's figures usable, so long as each result measures the feature and the control together.

`baseline` replaces only that feature's entries and records the commit, fingerprint and control they came from; `parity` names the commit in its table. It refuses results that carry no fingerprint, or more than one for a scenario, and results that do not all measure the feature and the same `search` scenarios. Commit the file with the change that needs it.

A baseline goes stale when the benchmark changes after the native implementation is gone, so the last commit that had it measures with the old benchmark. Branch from that commit, check out `scripts/latency.nu`, `scripts/latency/measure.nu` and `scripts/fixtures/clipboard.json` from your branch, give the feature's scenarios and the native `search` scenarios (the control) your branch's `prefix` and `typed` while keeping their native implementations, then commit, push, and dispatch the workflow with that commit as `ref`.

CI runs the Latency workflow as its `latency` job whenever the app could have changed (any pull request that touches more than `website/`), and on `main`. It uploads `results.json` as the `latency` artifact; a pull request's summary compares it with the latest from a successful CI run on `main` but does not fail on it, since a shared runner's noise is larger than many real changes. The parity check does fail the job, and the required `e2e` job fails unless `latency` passed, so a port that falls behind its native implementation can't merge. To see whether a reported regression is real, re-run the job, or run both commits on one quiet machine and compare those. Dispatch `latency.yml` on its own only to record a baseline, as above.

Scripts in this repository are written in Nushell.

## App size

CI's `build` job measures the `nix build .#dupin` bundle it already builds, the release profile (fat LTO, one codegen unit, stripped) that `scripts/package-app.nu` signs and ships, so no second build is made. `scripts/size.nu measure` records the bytes of `Contents/MacOS/dupin`, of the whole `Dupin.app`, of a zip made the way the release's `Dupin.zip` is (`ditto -c -k --keepParent`), and of the Resources: the bundled extensions (each one too), `ThirdPartyNotices.txt`, the EFF word list, the icons and everything else. The release copy differs from it only by its Developer ID signature and notarization ticket, a few KiB. CI uploads the figures from every build, on `main` and on pull requests, as the `size` artifact.

On a pull request, `scripts/size.nu report` compares them with those of the CI run on the `main` commit the pull request's merge commit sits on, its merge base with `main`, or, when that run measured nothing (it was cancelled, or hasn't finished), with the latest `main` run that did, and says which. The comparison goes to the job's summary, and the Size comment workflow (`size-comment.yml`) keeps one comment on the pull request up to date with it. Growth over 1% or 100 KiB is marked, but size is informational: nothing fails on it. The comment workflow runs on `workflow_run`, so it has a token that can comment even on a pull request from a fork; it runs only the default branch's code, and `report` reads the pull request's figures as data, refusing any that aren't whole byte counts under the names it knows. `main`'s figures are only recorded.

```sh
nix build .#dupin
nu scripts/size.nu measure result/Applications/Dupin.app --out target/size.json
nu scripts/size.nu report target/size.json                              # the sizes alone
nu scripts/size.nu report target/size.json --repo x-ryoppippi/dupin    # against main's (needs gh)
```

There is no per-crate breakdown: the shipped binary is stripped, and `cargo bloat --crates` would need a second fat-LTO build with symbols, which takes several times the minute it could cost.

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

## Third-party code and data

Dupin is GPL-3.0-or-later. The rule for anything from elsewhere:

- Borrowing an idea is fine: an algorithm, a ranking scheme (the frecency buckets in `dupin-core`'s history follow Firefox's), a feature another launcher has. Write the code yourself.
- Copying code or data (source, a ported shader, a word list, generated tables, fonts, icons) needs a licence compatible with GPL-3.0-or-later and attribution in the app's notices. Keep the original copyright and licence notice with the copy, and record it as below in the same pull request. Data under a licence `deny.toml` doesn't allow but `data-licences` in `packaging/third-party.toml` does (CC BY 3.0 and 4.0), which the app only reads, must ship as its own file in the app, never compiled into the binary. Without a licence, or under a non-commercial or no-derivatives one, don't copy it.

The app build writes `Dupin.app/Contents/Resources/ThirdPartyNotices.txt` (`packaging/notices.nu`, run by `package.nix`): the licence texts and copyright lines of every crate the binary links, as cargo-about reads them from the vendored crates, then each entry of `packaging/third-party.toml`. Settings → Acknowledgements opens it and `dupin --licenses` prints it. It is never committed, so it can't fall behind `Cargo.lock`.

- A new crate needs nothing unless CI says so. `checks.licences` runs `cargo deny check licenses` against the allow-list in `deny.toml`, and the notices build fails on the same list. A licence missing from it means checking that the licence is GPL-3.0-or-later compatible before adding it, or not taking the crate.
- When a crate ships no licence text with its copyright holders, cargo-about falls back to the licence's template and the notices build fails until the holders, from the crate's repository, are recorded under `[[crate]]` in `packaging/third-party.toml`. It also fails when such an entry is no longer needed.
- Anything else Dupin ships from elsewhere is a `[[material]]` entry in `packaging/third-party.toml`: what it is, where it came from, its licence, the licence text's path, and the files that hold it. `checks.licences` fails when a file under `crates/`, `extensions/`, `examples/shaders/` or `sdk/` names a copyright holder or a licence other than Dupin's and no entry lists it.

Run them locally with `nix build .#checks.aarch64-darwin.licences .#checks.aarch64-darwin.third-party-notices`.

## Logo

`assets/logo/dupin.svg` is the app icon and `assets/logo/dupin-mark.svg` a monochrome `currentColor` mark, for template images such as a menu-bar item. They are the only tracked logo sources: the Nix build draws every icon size from `dupin.svg` with resvg and packs them with icnsutil, so no generated PNG or icns is committed.

## Website

`website/` holds <https://dupin.app>, with its own pnpm project. Pages are Svelte components in `website/src/pages/` that [ox-content](https://github.com/ubugeeei-prod/ox-content)'s custom host (`website/src/host/index.ts`) renders to static HTML; the browser then hydrates the same component from the props written into the page, which runs the panel demo, the copy buttons and the download channel switch. The development shell provides Node.js and the pnpm version that `website/package.json` pins (`nix flake check` fails when they differ).

```sh
just website::install # dependencies from the lockfile
just website::dev     # local server, rendering the real pages
just website::preview # build, then serve the result with the redirects
just website::lint    # oxlint, type-aware and type-checked through tsrs
just website::deploy  # build and deploy the static site
```

Building and deploying go through `cf`, with the Cloudflare Vite plugin producing the Build Output. Workers Builds runs `pnpm run deploy` on `main` (the `dupin` Worker on `dupin.app`) and `cf previews deploy` on other branches, whenever `website/` or `assets/logo/` changes. The site is static assets only, with no Worker code: HTTP reaches HTTPS through the `dupin.app` zone's Always Use HTTPS setting, and `dupin.app/gh` redirects to the repository through `_redirects`, declared in `website/vite.config.ts`.

Icons are `icon-[set--name]` classes that ox-content turns into one self-hosted stylesheet from the `@iconify-json/*` collections installed in `website/`; add the collection before using a new set. For agents and crawlers the site also serves `/llms.txt`, a Markdown twin of each page (`/index.md`, `/download.md`), `sitemap.xml` and `robots.txt`; the Markdown is built in `website/src/markdown.ts` from the same data as the pages, and `website/public/_headers` sets their content types.

The shared page head supplies canonical URLs, social cards and application JSON-LD.
The 1200 × 630 social image is generated from `assets/social/og.svg`, which reuses
the app logo. After editing the source, regenerate `website/public/og.png`:

```sh
nix shell --inputs-from . nixpkgs#resvg --command resvg --skip-system-fonts \
  --use-fonts-dir "$(nix build --inputs-from . --no-link --print-out-paths nixpkgs#inter)/share/fonts" \
  assets/social/og.svg website/public/og.png
```
