<div align="center">
    <img src="./assets/logo/dupin.svg" alt="Dupin logo" width="128" height="128">
    <h1>Dupin</h1>
</div>

<p align="center">
    <a href="https://dupin.app">dupin.app</a>
</p>

<div align="center">
    <a href="https://dupin.app"><img src="./docs/public/website.png" alt="dupin.app: Press ⌘Space. It's there." width="720"></a>
</div>

> A lightweight, resident launcher for Apple silicon Macs.

- Fuzzy-search and open apps, by any name macOS knows them by
- Space on an empty query searches files with Spotlight; `in` and Space search inside them; `define` and Space looks up a word in macOS dictionaries
- A hotkey (⌘Space by default) shows the panel at once, on Liquid Glass
- An optional shader animates behind the panel and reacts to typing; bring your own WGSL, as with Ghostty's custom shaders ([examples](./examples/shaders))

## Quick start

Free ⌘Space first: turn off Spotlight's shortcut and unassign it in any other launcher ([how](./docs/configuration.md#freeing-the-hotkey)). Then install it with Homebrew (`dupin@nightly` for builds of `main`); it [updates itself](./docs/configuration.md#updates):

```sh
brew install --cask x-ryoppippi/tap/dupin
```

Or build it with Nix:

```sh
nix build .#dupin
open result/Applications/Dupin.app
```

## Documentation

See [docs/](./docs/README.md) for using, configuring and developing Dupin.

## Major Sponsors

<p align="center">
    <a href="https://coderabbit.link/ryoppippi">
        <picture>
            <source media="(prefers-color-scheme: dark)" srcset="./docs/public/coderabbit-logo-dark.svg">
            <img src="./docs/public/coderabbit-logo.svg" alt="CodeRabbit" width="320">
        </picture>
    </a>
    &nbsp;&nbsp;&nbsp;&nbsp;
    <a href="https://blacksmith.sh">
        <picture>
            <source media="(prefers-color-scheme: dark)" srcset="./docs/public/blacksmith-logo-dark.svg">
            <img src="./docs/public/blacksmith-logo.svg" alt="CI powered by Blacksmith" width="320">
        </picture>
    </a>
</p>

## GitHub Sponsors

<p align="center">
    <a href="https://github.com/sponsors/ryoppippi">
        <img src="https://sponsors.ryoppippi.com/sponsors.png" alt="Sponsors">
    </a>
</p>

## License

Dupin is licensed under [GPL-3.0-or-later](./LICENSE). Contributions need the [Contributor License Agreement](./CLA.md); see [CONTRIBUTING.md](./CONTRIBUTING.md).

## Local CLI

With a resident Dupin instance, `dupin open --query 'Safari'` shows the panel without
activating a result. `printf 'red\nblue\n' | dupin pick` opens a transient picker;
Enter prints the original value and Esc exits unsuccessfully. For structured values,
use `dupin pick --input-json` with a JSON array. Values are displayed as data, including
objects containing paths or action names. Picker sessions never add history entries.

`dupin config get --json` returns the current configuration. Save a copy as `base.json`,
edit a second copy, then run `dupin config set base.json desired.json --json`.
The base enables conflict detection against external edits. Changes use the settings
service, including managed/read-only files and native shortcut validation; a successful
reply follows reconciliation. `--json` emits JSON values on success and an `error` object
on failure. `dupin --help` and `dupin --version` perform no app or configuration work.

Pure headless examples (no resident instance required):

```sh
printf '%s' '{"kind":"text","title":"Hello","subtitle":"Headless fixture"}' > /tmp/dupin-view.json
dupin snap /tmp/dupin-view.json /tmp/dupin-view.png --json
dupin ext check fixture.wasm request.json --json
dupin ext snap fixture.wasm request.json /tmp/dupin-extension.png --json
```

The extension commands require a build with the optional `extension-tools` Cargo
feature; ordinary builds do not link the Wasm runtime. They accept the bounded
versioned guest request contract, execute a
module through the existing fuel/deadline-limited runtime, and validate its response.
Imports (including OS/WASI imports) are forbidden. This is a guest-contract tool, not an
extension loader. Headless rendering shows the returned text/detail/pair views without
executing actions or loading configuration, history or application providers.

Local control uses length-prefixed JSON, one correlated request/reply per connection,
a 512 KiB frame cap, at most eight live connections and a five-minute request deadline.
The socket lives in an owner-only Dupin directory under `XDG_RUNTIME_DIR` (or the system
per-user temporary directory on macOS, independent of shell `TMPDIR`). A live instance
is never replaced; an owner-only socket with no listener is recovered on restart.
Picker and settings contention return Busy. Disconnecting cancels a pending picker.
Use `dupin snapshots list --json` for saved Snapshot metadata, `dupin snapshots open
[--id ID]` to show the separate window, and `dupin snapshots close` to hide it.
The list includes only id, saved time, command, title and query; it reports archive
problems and marks truncated replies. These commands never save, delete, resume a
query, activate a result or change history. Opening is refused while a picker is active.
