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
just render      # headless renders (see Headless renders)
just docs-images # the images in docs/images (see Documentation images)
just docs-glass  # the home page's Liquid Glass clips (see Liquid Glass clips)
just footprint   # the peak-footprint budget (see Peak memory)
```

`direnv allow` (or `nix develop`) gives a shell with cargo, clippy, rustfmt, rust-analyzer, nushell and the formatters:

```sh
cargo run -p dupin
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p dupin-mac --example search -- in invoice # a provider from the terminal: apps, files or in
```

Dev builds keep full debug info for the workspace's own crates only; dependencies carry line tables, so backtraces still name their files and lines. To profile, build with `cargo build -p dupin --profile profiling`: the release build's code, with full debug info and its symbols kept, in `target/profiling/dupin`.

For rebuild times and the sccache decision, see [Build caching](#build-caching).

## Remote development with Testbox

The development shell includes Blacksmith CLI 0.5.3. The manually dispatched
`testbox-dev.yml` workflow compiles native test dependencies on a disposable Apple
silicon runner, then keeps it available for commands. It follows ccusage's
Testbox setup and uses the same macOS runner and Nix setup as CI.

CI jobs that do any real work (build, test, lint, format, record) run on the largest
Blacksmith instance for their OS; jobs that only post comments may use `ubuntu-slim`. Check
https://docs.blacksmith.sh when adding a job or when Blacksmith adds larger runners.
On macOS that is `blacksmith-12vcpu-macos-26`, not the same-sized macOS 27 runner, because
27 is slower per core. At one commit and one cache state, CI's `build` job took 9–14% longer on
12 vCPU macOS 27 than on 26, the release build's fat LTO step 12–17% longer and the
extension-host test build 18–19% longer. `latency`'s parity does not hold on 27 either, but
not because extensions are slower there: on 12 vCPU runners an extension's whole host call
(the socket, the sandboxed helper, QuickJS and the host service) took 560–653 µs at p95 on 27
and 639–675 µs on 26. A headless run takes a delivery only when its next frame starts, and
rune paces those frames with `thread::sleep`, whose sub-millisecond wake-ups on the 27 runners
are late by anything from 0 to 1.1 ms where on 26 they are late by a nearly constant 0.9–1 ms.
That jitter lands in `merge_ms`, so it widens the p95 of scenarios whose own work is short, as
the extensions' is, more than the native `search` control's, and the control's scale can't
absorb it ([Keypress latency](#keypress-latency)). A window wakes on the delivery instead, so
this is the benchmark's, not the app's. Measure again before moving to a newer macOS.

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
times representative edits. On the 12-CPU macOS 27 Testbox, an incremental
`cargo build -p dupin` after editing a UI file, `dupin-core`'s session code or
`dupin-mac` takes 2.8–3.6 seconds, `cargo test --workspace --no-run` 3.1–4.9 and
`cargo clippy` 1.4–2.8. A docs-only edit rebuilds nothing. Alternating `build`, `test` and
`clippy` rebuilds nothing either, so no feature difference between them
forces rebuilds. A release build takes 126–129 seconds whichever crate changed,
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
compiled dependencies. On the 12-CPU Testbox, the same step after a UI edit
takes 163 seconds, and the three dependency outputs that step builds together
hold 1.19 GB (debug 406 MB, release 384 MB, Hawk 401 MB).

sccache is not configured for local builds or CI. Cargo builds workspace
crates incrementally in the dev profile, which sccache cannot cache, and those
crates are all that an edit rebuilds. It can only replay third-party crates in
a fresh target directory. In the
[Cargo cache experiment](../experiments/cargo-cache/README.md) on the 12-CPU
Testbox, a warm cache built a fresh target at a new path, as a new worktree has,
in 47.5 seconds against 47.0–47.4 without it, and a cold cache cost 6.8 seconds
more. It only pays when a removed target is rebuilt at the same path: 21.9
seconds against 47.3. CI compiles through Nix, which caches whole dependency outputs
instead. The development shell does not include sccache. A developer who often
deletes and rebuilds the same target directory can run it from nixpkgs without
installing it:

```sh
nix build --inputs-from . nixpkgs#sccache --out-link target/sccache
RUSTC_WRAPPER="$PWD/target/sccache/bin/sccache" cargo build
```

## Formatting

`nix fmt` runs treefmt with nixfmt, deadnix and statix for Nix, rustfmt, oxfmt (configured in `.oxfmtrc.json`) for JSON, YAML, TOML and Markdown, nufmt for Nushell, typos (`typos.toml`; it only reports, since a typo's fix needs a person), and actionlint and zizmor for the workflows. `nix flake check` fails on anything left unformatted.

Entering the development shell installs Git hooks (`nix/git-hooks.nix`, run by prek): treefmt and `gitleaks protect` on each commit, and `treefmt --fail-on-change` and `gitleaks detect` on each push. The hooks and their config live in the repository's common Git directory, so one shell entry from any worktree covers every worktree of the clone, including those created later with `git worktree add`. Never skip them with `PREK_ALLOW_NO_CONFIG` or `--no-verify`; if a commit reports a missing config, enter the development shell once.

On every pull request, the `autofix.ci` workflow runs `nix fmt` after the [Rune step](#rune-updates), and autofix.ci commits both fixes in one commit, so unformatted files come back formatted instead of failing CI. The flake targets only aarch64-darwin, but it exports the same treefmt configuration as `formatter.x86_64-linux`, so the workflow runs on a cheap Linux runner. The `treefmt` check in CI still fails where autofix.ci can't commit, such as a pull request from a fork that hasn't installed the app, and on what the formatters report but can't fix, such as a typo, an actionlint error or a zizmor finding. The workflow also runs on every push to `main`, without formatting or committing anything, because Blacksmith saves the job's Nix store only from `main`; without it, every pull request would download the formatter, Nushell and cargo again.

## Headless renders

Every build of Dupin, the released app included, takes Rune's headless flags (`--snap`, Rune's own name for writing the frame to a PNG, `--tree`, `--script`, `--realtime`, ...; see Rune's `crates/rune/README.md`, "Agent tooling"). `scripts/render.nu` renders every scenario in `scripts/headless/` to `target/screenshots/`:

```sh
cargo build -p dupin
nu scripts/render.nu
nu scripts/render.nu --only calendar --theme dark # one scenario, in one appearance
```

`--theme light` or `--theme dark` fixes the appearance and adds it to each file name; without it the renders follow the Mac's. The renders show real results from the machine, file paths included, so they stay out of the repository. Calendar and dictionary scenarios use fixed fixtures instead, and clipboard scenarios the history in `scripts/fixtures/clipboard.json`, or a scenario's own in `<name>.clipboard/history.json`, whose `image` entries name PNG files beside it. A scenario with a `<name>.snapshots/snapshots.json`, a list of snapshots, starts with them saved in scratch storage. A scenario with a `<name>.home/` folder runs with a copy of it as its home: apps in its `Applications` folder are found like any other, and its `Library/Application Support/Dupin/history.json` is the usage history, where `$HOME` in an app's path stands for the copy's path. A scenario's own snapshots, clipboard history, extensions, grants and home are read only beside its own `<name>.toml`. Headless renders draw the panel's plain card where a window shows Liquid Glass.

Screenshots for a PR: reference local files in the body and pass them with `gh pr create/edit --attach`; never commit them. gh rewrites only Markdown references (`![alt](./shot.png)`), not an HTML `<img src>`.

## Documentation site

dupin.app/docs is this folder, rendered when the website builds (`just website::build`): `website/src/docs/pages.ts` lists the pages it publishes, by section, and ox-content renders each one and indexes `docs/` for the site's search. A link to another published page goes to its page on the site, and a link to anything else, such as this page, a design document or `extensions/`, goes to GitHub. Add a new user-facing page to `pages.ts`; pages for people working on Dupin stay on GitHub only.

## Documentation images

The images in `docs/images` are rendered, never taken by hand. `scripts/docs-images/images.toml` lists each one; `just docs-images` (`nu scripts/docs-images.nu` after `cargo build -p dupin`) renders every one in light and dark to `docs/images/<page>/<name>-<theme>.png`, compresses them losslessly with oxipng and deletes the images the list no longer has. `--only <text>` renders just the images whose `<page>/<name>` contains it. Re-run it after a change to anything an image shows, and commit the result.

Every image comes from fixtures, so a render on any Mac shows nothing of that Mac's: a bundled or example extension's own fixture through `dupin ext render`, or a headless scenario run in a scratch home that holds the fixture documents and usage history the list names, with the fixture clipboard history and snapshots, and with `DUPIN_HEADLESS_APPS=/System/Applications`. A headless run takes that `PATH`-style list in place of the app folders, so it lists only the stock apps; `dupin ext render` sets it empty. It treats as running only the app bundles `DUPIN_HEADLESS_RUNNING` lists the same way, none without it, so an action offered only while its app runs shows the same on every Mac. Neither inherits the shell's environment: each run gets only the variables it is given, so one set on the rendering Mac, such as `DO_NOT_TRACK`, changes nothing drawn. Spotlight indexes nothing in a scratch home, and on some Macs not even a fresh disk image, so no image shows a Spotlight search: the file images start from Recent items. Settings pages are drawn from the `prefs` window (`--window prefs`).

A page embeds an image as a `<picture>` whose `prefers-color-scheme: dark` source is the dark image, so GitHub shows the one matching the reader's theme:

```html
<picture>
	<source media="(prefers-color-scheme: dark)" srcset="../images/calendar/upcoming-dark.png" />
	<img alt="What the image shows" src="../images/calendar/upcoming-light.png" width="680" />
</picture>
```

Pixels differ from Mac to Mac (fonts, app icons and the GPU), so CI doesn't render the images again. `checks.docs-images` runs `nu scripts/docs-images.nu check` instead: every listed image and clip exists in each of its themes, `docs/images` holds nothing else, some page or the website embeds each one and every image either embeds exists.

### Clips

The same list's `[[clips]]` are short recordings: each plays its `steps` (typed text, keys and waits) into the panel in a headless `--realtime` run, in the same scratch home as the images, and saves `docs/images/<page>/<name>-<theme>.webp` in light and dark. `just docs-clips` (`nu scripts/docs-images.nu --kind clips`) renders only the clips; `just docs-images` renders both. Rune writes the frames at 10 fps and 1.25× scale, each clip is cut to the lowest edge the panel reaches in any frame, and a caption below the panel names each key pressed (`⇥ Tab`, `⌘K`) or the text typed so far (`type: cal`). ImageMagick draws the captions in Inter, the same on every Mac, which the dev shell provides as `DUPIN_CAPTION_FONT`, and `img2webp` encodes the frames as a lossy animated WebP, in which a frame that repeats the last costs nothing. A clip is embedded like an image, as a `<picture>` of its two WebP files:

```html
<picture>
	<source media="(prefers-color-scheme: dark)" srcset="images/tutorial/apps-dark.webp" />
	<img alt="What the clip shows" src="images/tutorial/apps-light.webp" width="680" />
</picture>
```

Animated WebP, not video, because the same Markdown is read on GitHub, which shows a `<video>` only for files uploaded to an issue or pull request, never one in the repository; GitHub shows an animated WebP as an image, and so does every current browser, without a player or autoplay rules. A clip must never type into a Spotlight search (Space on an empty query, or `in`), since Spotlight answers from the rendering Mac's own index; the files clips start from Recent items and folder browsing instead, which read only the scratch home.

The website's home page plays clips of its own, on page `home`: one per tab of its demo (Apps, Files, In files, Dictionary, Keywords and Clipboard) over Liquid Glass. A page outside GitHub can play video, which takes a fraction of an animated WebP's bytes for a backdrop that changes every pixel of every frame, so these set `video = true`: `ffmpeg` encodes the frames over the page's black (`background`) as an H.264 MP4, and the last frame is saved as `<name>-<theme>-poster.webp`, which the page shows before the video plays and in its place under reduced motion. `themes = ["dark"]` renders only the theme the site uses, `scale` sets the scale (2, so their text stays sharp on a Retina screen), `backdrop` names the shader a headless stand-in is drawn over, as `config.toml` would, and `captions = false` leaves the captions off. The website names each file at the path it serves it from (`/docs/images/home/apps-dark.mp4`), and `check` counts those as embeds.

### Liquid Glass clips

A headless render draws Liquid Glass as an opaque fallback, since there is no desktop behind it to refract, so the clips marked `glass = true` are recorded from the real app instead. `just docs-glass` (`nu scripts/docs-glass.nu --app result/Applications/Dupin.app` after `nix build .#dupin`) needs macOS 26 or later in Dark mode, and a terminal already allowed Screen Recording and Accessibility; it never asks for either. For each clip it starts the bundle's `dupin` as an instance of its own, with the scratch home and configuration the headless clips use, `DUPIN_RECORDING=1`, no hotkey (so a Dupin already running keeps its own), its own `XDG_RUNTIME_DIR` and telemetry off, and nothing else of the shell's environment; covers the screens with a borderless window showing the wallpaper, so the desktop picture is never changed; records the screen with ffmpeg; opens the panel with `dupin open`; presses the clip's steps at their times through System Events; and cuts the recording to the panel and the wallpaper around it, with the same captions below, to the files `just docs-clips` would render. Its log, scratch home and raw recording stay in `target/docs-glass` until the next run. `DUPIN_RECORDING=1` makes the real app draw on fixtures as a headless run does: it scans only the folders `DUPIN_HEADLESS_APPS` names, and opens its storage, where the scratch home's usage history is migrated to, under a throwaway key instead of the Keychain, which a scratch home has no item in. It takes effect only when the launch's `HOME` is not the account's own, so a normal launch, whose `HOME` is, ignores a stray variable and never hides the user's apps or history. Its clipboard history is the fixture's, seeded into the scratch storage, and it never watches the real pasteboard, which a recording must not be allowed to read.

The wallpaper is the manifest's `wallpaper`, a clip's own `wallpaper`, or `--wallpaper` for every clip: a preset drawn by ImageMagick, each a landscape of a sky, a sun or moon, layered mountain ridges, a treeline and a lake (`dusk`, the default, `lagoon` for a day and `night`), whose edges and contrasts the glass visibly blurs and bends or the path to an image. Only wallpapers made here are published; macOS's own are Apple's.

`docs/images/glass.json` lists each clip recorded this way, with the macOS version, wallpaper and screen scale. `just docs-clips` keeps those and renders the rest headless over their `backdrop` (Flow) in their place, and the website says which tabs are headless recordings over Flow. `check` fails if it lists a clip that isn't `glass = true`.

The `Docs glass` workflow (`.github/workflows/docs-glass.yml`) runs the same on a `blacksmith-6vcpu-macos-latest` runner, which runs macOS 26 with a GUI session and grants its runner Screen Recording and Accessibility, as the E2E job relies on. Dispatch it (optionally with `only` or `wallpaper`), then `gh run download <run> --name docs-glass --dir docs/images` and commit the result. A pull request that changes the recorder runs it too. The runner's own display is a 1920×1200 framebuffer at 1× with no 2× mode, so on CI the recorder adds a 1440×1000 virtual display, tall enough for every clip to be cut to the demo frame's shape, at 2× (the private `CGVirtualDisplay`, which needs no permission), makes it the main display and mirrors the runner's to it, so the clips are as sharp as a Retina Mac's. It does this only when `CI` is `true`, never on a Mac in use. The runner also has Reduce Transparency on (`defaults read com.apple.universalaccess reduceTransparency` prints `1`), which makes Liquid Glass a solid panel, so on CI the recorder turns it and Increase Contrast off for the job; on a Mac in use it only reports them.

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

Each scenario in `scripts/latency/scenarios.toml` names a feature, an unmeasured prefix (`cal `) and the characters measured one keypress at a time (`rev`). Each implementation of a feature names the provider that answers it and, for an extension, the `package` behind it (`bundled:<name>` for one that ships with Dupin); the run fails when a measured keypress does not search that provider answered by that package, or by Dupin itself for an implementation without one, or the whole of the measured characters shows no rows from it, so a scenario that never reaches its feature cannot pass as a measurement of it. A headless run reaches no program, so a package whose catalogue runs one, as Processes does, is measured against a fixture: its implementation's `extensions` folder holds only `.headless-fixture.json`, `{ "name": <bundled package>, "fixture": <file beside it> }`, whose replies, `exec` included, answer every host request and whose consent turns the package on, as `dupin ext render` hands a fixture over. A package that changes a feature it doesn't answer, such as one adding default kind actions to the main search's app rows, is measured by a variant of that feature's native implementation that lists it in `enables`: the run turns it on in the broker's base (`extension-grants.toml` beside the config), the provider is still answered by Dupin itself, and every pass must end with rows from each enabled package or carrying its kind actions, which a headless run's latency samples list as `composed`, or the run fails. `parity` holds such a variant to the native implementation of the same run, as `search.running-apps` is held to `search`. For a built-in command a bundled extension answers, only the rows from that package count as the provider's: the command shows a row of its own when its package is off, missing or failing, and that row is cheap enough to pass any budget. A run types the scenario `--warmup` times unmeasured, then until `--samples` keypresses (210 by default) are measured, `--gap` apart. Each scenario runs `--runs` times (three by default), each in a fresh process, and every reported figure is the median of the runs' own, so one run disturbed by the rest of a shared machine does not move the result. A keypress typed over before its results were drawn has no latency; the run fails when more than 1% are, as the gap is then too short.

For each scenario the results give p50, p95 and p99 of the total, and p50 and p95 of each stage, which add up to it:

- `dispatch_ms`: from the edit to the searches it starts being handed to their providers' workers.
- `provider_ms`: from there to the last provider's final delivery leaving its worker. `providers` in the raw lines has each one's time.
- `merge_ms`: from there to the session holding the finished, ranked results. It includes waiting for the UI thread. A headless run's UI thread takes deliveries only when its next frame starts, and frames start at most every `1 / --fps` seconds, paced by `thread::sleep`, so this includes up to a millisecond of waiting at the default 1000 fps, plus however late the sleep wakes.
- `layout_ms`: from there to the next frame being built, laid out and encoded.

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

A headless run has no display, so the panel lays out its default 416-point list (eight rows). An implementation's `display` simulates one: `run` sets `DUPIN_HEADLESS_DISPLAY` to it, `<width>x<height>` points of usable area, and the panel sizes its list from that area as it would from a real display's, through the same placement. `search-tall` measures the main search on a 1728×2560 display, whose list reaches the 832-point maximum (16 rows), so drawing more rows can't slow every keypress unnoticed. `display` is set outside `scripts/latency/measure.nu`, so it doesn't change any scenario's `benchmark` fingerprint.

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

CI runs the Latency workflow as its `latency` job whenever the app could have changed (any pull request that touches more than `docs/`, `experiments/`, `website/`, `telemetry/` and the top-level Markdown files; see the `changes` job in `.github/workflows/ci.yml`), and on `main`. It uploads `results.json` as the `latency` artifact. The parity check fails the job, and the required `e2e` job fails unless `latency` passed, so a port that falls behind its native implementation can't merge. Dispatch `latency.yml` on its own only to record a baseline, as above.

A pull request is compared with `main` but never fails on it, since a shared runner's noise is larger than many real changes and a slowdown can be worth what it buys. `scripts/latency.nu fetch-main` downloads the results of the CI run on the `main` commit the pull request's merge commit sits on, or, when that run measured nothing, of the latest `main` run that did (the same choice as the size report's, in `scripts/main-build.nu`), and the job uploads them in its `latency` artifact as `main.json`. `scripts/latency.nu report` holds each scenario to them with `compare`'s tolerance, 10% or 1 ms of main's p95, whichever is larger, after multiplying main's figures by this run's median native `search` p95 over main's, as `parity` scales a recorded baseline, so a slower runner does not read as a slower pull request. The `search` scenarios themselves are held to main's unscaled, since the scale is made of them; when they are slower, the comment says that scaling may hide a slowdown every keypress pays for.

The comparison goes to the job's summary, and the Latency comment workflow (`latency-comment.yml`) keeps one comment (`<!-- dupin-latency -->`) on the pull request up to date with it. When nothing is slower it is one line, with every scenario's figures collapsed. When something is, it names each slower scenario with its change in milliseconds and percent and the stage most of it is in, shows each stage's p95 against main's, and says what to do: look at the pull request's changes on the per-keystroke path, starting with that stage; re-run the job to rule out noise; measure both commits on one machine with `run --only <feature>` and `compare`; or say in the pull request why the slowdown is intended. A collapsed JSON block repeats the slower scenarios, their deltas and stages, for an agent to parse. Like the size comment, the workflow runs on `workflow_run`, executes only the default branch's code, and `report` refuses results whose names are not scenario names or whose figures are not times.

Agents working on or reviewing a pull request should read its latency comment and investigate any scenario it flags before calling the work done.

```sh
nu scripts/latency.nu fetch-main x-ryoppippi/dupin <commit> --out target/latency/main.json   # needs gh
nu scripts/latency.nu report target/latency/results.json target/latency/main.json
```

Scripts in this repository are written in Nushell.

## App size

CI's `build` job measures the `nix build .#dupin` bundle it already builds, the release profile (fat LTO, one codegen unit, stripped) that `scripts/package-app.nu` signs and ships, so no second build is made. `scripts/size.nu measure` records the bytes of `Contents/MacOS/dupin`, of the whole `Dupin.app`, of a zip made the way the release's `Dupin.zip` is (`ditto -c -k --keepParent`), and of the Resources: the bundled extensions (each one too), `ThirdPartyNotices.txt`, the EFF word list, the icons and everything else. The release copy differs from it only by its Developer ID signature and notarization ticket, a few KiB. CI uploads the figures from every build, on `main` and on pull requests, as the `size` artifact.

On a pull request, `scripts/size.nu report` compares them with those of the CI run on the `main` commit the pull request's merge commit sits on, its merge base with `main`, or, when that run measured nothing (it was cancelled, or hasn't finished), with the latest `main` run that did, and says which (`scripts/main-build.nu`, which the latency comparison shares). The comparison goes to the job's summary, and the Size comment workflow (`size-comment.yml`) keeps one comment on the pull request up to date with it. Growth over 1% or 100 KiB is marked, but size is informational: nothing fails on it. The comment workflow runs on `workflow_run`, so it has a token that can comment even on a pull request from a fork; it runs only the default branch's code, and `report` reads the pull request's figures as data, refusing any that aren't whole byte counts under the names it knows. `main`'s figures are only recorded.

```sh
nix build .#dupin
nu scripts/size.nu measure result/Applications/Dupin.app --out target/size.json
nu scripts/size.nu report target/size.json                              # the sizes alone
nu scripts/size.nu report target/size.json --repo x-ryoppippi/dupin    # against main's (needs gh)
```

There is no per-crate breakdown: the shipped binary is stripped, and `cargo bloat --crates` would need a second fat-LTO build with symbols, which takes several times the minute it could cost.

## Search quality

Keypress latency measures how fast the results arrive; search quality measures whether they are the right ones, as the distance from intent to result: how many keystrokes a person needs before the row they want is first. `scripts/search-quality.nu` measures it with `dupin-search-quality`, a benchmark that types each case into a launcher `Session` built as the app builds its main search, so it ranks through the same code: the apps source (`dupin_mac::AppsProvider`, here over fixture apps, with the native fuzzy matcher), the system commands, the history's boosts, the rows that name keywords, web searches and aliases, and how the session merges providers' rows. It needs no UI and no permissions, and it is deterministic: every case runs at the same fixed time.

```sh
cargo build -p dupin-search-quality                                    # or nix build .#dupin-search-quality
nu scripts/search-quality.nu run                                       # target/search-quality/results.json
nu scripts/search-quality.nu compare main.json target/search-quality/results.json
nu scripts/search-quality.nu report target/search-quality/results.json --repo x-ryoppippi/dupin   # against main's (needs gh)
```

`scripts/search-quality/corpus.toml` holds the fixtures, none of them read from a real Mac: a stock macOS install's apps and a plausible set of third-party ones chosen for names that compete for the same letters (Arc, Ardia and Arto; AppCleaner, Activity Monitor and Alfred 6), the other names Spotlight knows an app by (System Settings as System Preferences, Music as Apple Radio), the commands keywords enter (`cal`, `d`, `clip`, `emoji`, `snap`, …), web search aliases, `[aliases]` for apps, and named histories of picks: `base`, a heavy user's month, and smaller ones for particular scenarios, such as the report of #300. The web search's own row stands in for the bundled web-search extension, which the benchmark leaves out: it only has to take the pinned first place a typed alias gives it.

Each case in `scripts/search-quality/cases.toml` names its `intent` (the row wanted: `app:<name>`, `command:<id>`, `web:<alias>` or `system:<title>`), the `query` a person would type for it, the `history` it runs with (`base` unless it says otherwise, `[]` for a fresh install) and `tags`. The benchmark types the query one character at a time and records where the row is after each. A case records how Dupin ranks today, good or poor: a case that never reaches first, such as initials (`vsc` for Visual Studio Code), is a known weakness to improve, not an error. A case the corpus can't answer (an unknown app, command, web search or history) fails the run, as does a corpus that names an app it doesn't list; the crate's test runs the repository's cases, so that also fails `cargo test`.

From each case's ranks the script derives the keystrokes until the row is first, whether it ever drops out of the 8 rows the panel shows (`VISIBLE_ROWS`) after being in them, and, over all cases, the mean keystrokes until first (a case never first counts as its whole query and one more), the share first within 1, 2, 3 and 5 keystrokes, and the hardest cases.

CI's `search-quality` job builds `.#dupin-search-quality` on the checks' dependency build whenever the app could have changed, runs every case, and uploads `results.json` as the `search-quality` artifact, on `main` and on pull requests. A regression never fails the job; the benchmark itself failing does, and the required `e2e` job fails unless `search-quality` passed, so a pull request can't break the benchmark. On a pull request, `report` compares the results with those of the CI run on the `main` commit the pull request's merge commit sits on, or, when that run measured nothing, of the latest `main` run that did (`scripts/main-build.nu`, as the size and latency reports do). A case regresses when it needs more keystrokes than on main or its row is lower after any keystroke, and improves when it needs fewer or is higher after one and lower after none; a case whose intent, query or history changed is compared with nothing, and the report says when the corpus or cases changed at all.

The comparison goes to the job's summary, and the Search quality comment workflow (`search-quality-comment.yml`) keeps one comment (`<!-- dupin-search-quality -->`) on the pull request up to date with it: the summary line against main's, the cases that regressed with their ranks on main and here and what to do, then the hardest cases, every case, and a JSON block of the same for an agent to parse, collapsed. Like the size and latency comments, the workflow runs on `workflow_run`, executes only the default branch's code, and `report` refuses results whose names, queries or intents are not ones a case could have or whose ranks are not ranks.

Agents working on or reviewing a pull request that touches ranking, matching, the history or the session's merging should read its search-quality comment and explain any case it flags before calling the work done. When a change is meant to trade one case for another, or a case no longer asks for what a person would want, change `cases.toml` in the same pull request and say why; keep a case's `id` while its intent stays the same, so it is compared with main's.

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
`dupin-telemetry` is excluded for the same reason: its `Ping` is the wire contract
with the collector in `telemetry/`, which reads every field through the generated
schema rather than through Rust.

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

`website/` holds <https://dupin.app>, with its own pnpm project. Pages are Svelte components in `website/src/pages/` that [ox-content](https://github.com/ubugeeei-prod/ox-content)'s custom host (`website/src/host/index.ts`) renders to static HTML; the browser then hydrates the same component from the props written into the page, which runs the home page's recordings, the copy buttons and the download channel switch. The development shell provides Node.js and the pnpm version that `website/package.json` pins (`nix flake check` fails when they differ).

```sh
just website::install # dependencies from the lockfile
just website::dev     # local server, rendering the real pages
just website::preview # build, then serve the result with the redirects
just website::lint    # oxlint, type-aware and type-checked through tsrs
just website::deploy  # build and deploy the static site
```

Building and deploying go through `cf`, with the Cloudflare Vite plugin producing the Build Output. The `Website` workflow builds and lints the site on every pull request that changes `website/`, `docs/`, `assets/logo/` or `VERSION`, and deploys it with `just website::deploy` (the `dupin` Worker on `dupin.app`) when such a change reaches `main`; see [Deployment](#deployment). The site is static assets only, with no Worker code: HTTP reaches HTTPS through the `dupin.app` zone's Always Use HTTPS setting, and `dupin.app/gh` redirects to the repository through `_redirects`, declared in `website/vite.config.ts`.

Icons are `icon-[set--name]` classes that ox-content turns into one self-hosted stylesheet from the `@iconify-json/*` collections installed in `website/`; add the collection before using a new set. For agents and crawlers the site also serves `/llms.txt`, a Markdown twin of each page (`/index.md`, `/download.md`), `sitemap.xml` and `robots.txt`; the Markdown is built in `website/src/markdown.ts` from the same data as the pages, and `website/public/_headers` sets their content types.

The shared page head supplies canonical URLs, social cards and application JSON-LD.
Every page gets its own 1200 × 630 social image, drawn at build time by Takumi in `website/src/host/og.ts` from the page title (the home page shows the tagline) over the app logo, with Inter from `website/src/host/fonts/`. The cards are served at `/og/<page>.png`, and each page's `og:image` adds `?v=` and a hash of the card's pixels, so link previews fetch a changed card instead of keeping the cached one. To change the design, edit `markup` in `og.ts`; there is no image to regenerate.

## Deployment

GitHub Actions deploys both Cloudflare Workers from `main`, never from a developer's Mac:

| Workflow    | Runs when `main` changes                       | Deploys                                                              |
| ----------- | ---------------------------------------------- | -------------------------------------------------------------------- |
| `Website`   | `website/`, `docs/`, `assets/logo/`, `VERSION` | `dupin.app`, with `cf deploy` (`just website::deploy`)               |
| `Telemetry` | `telemetry/`, `assets/logo/dupin.svg`          | `telemetry.dupin.app`, with `void deploy` (`just telemetry::deploy`) |

Both can also be run by hand from **Actions**. One run of each deploys at a time, and a deployment that started finishes. Until the credentials below exist, a run builds and tests as usual, skips the deployment and says so in a notice, so CI stays green.

They read these from the repository's **Settings → Secrets and variables → Actions**:

- `CLOUDFLARE_API_TOKEN` (secret): an API token created from the **Edit Cloudflare Workers** template, limited to Dupin's account (**Account Resources**) and the `dupin.app` zone (**Zone Resources**). It grants Workers Scripts, Workers KV Storage and Workers Routes edit; add **Zone → DNS → Edit** so the first telemetry deployment can attach the `telemetry.dupin.app` custom domain.
- `CLOUDFLARE_ACCOUNT_ID` (secret): the account ID `website/cloudflare.config.ts` and `telemetry/void.config.ts` name.
- `DUPIN_TELEMETRY_KV_NAMESPACE_ID` (variable, not secret): the ID of a KV namespace named `dupin-telemetry-rollups`, created once under **Storage & Databases → Workers KV**. The collector's daily rollups live there.

The Analytics Engine dataset `dupin_telemetry` is created by the Worker's first write and needs no setup. Workers Builds is no longer needed for `dupin.app`: disconnect it in the `dupin` Worker's **Settings → Build** once the `Website` workflow has deployed, so the two don't both deploy each merge.
