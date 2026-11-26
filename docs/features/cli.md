# Command line

The `dupin` binary inside Dupin.app is also its command-line client. With the Nix package or the home-manager module it is on `PATH`; otherwise run `/Applications/Dupin.app/Contents/MacOS/dupin`.

It is how tools outside Dupin drive it: a terminal, a shell script or an AI agent opens the panel, asks you to pick, reads snapshots or changes the configuration. [Extensions](./extensions.md) are different: they run inside Dupin, sandboxed, and answer a keyword in the panel.

<WaysIn>

Three ways in: you use the panel; extensions run inside Dupin, sandboxed; scripts and agents drive Dupin from outside with `dupin` and `dupin mcp`.

</WaysIn>

## The panel

With a resident Dupin instance, `dupin open --query 'Safari'` shows the panel without activating a result; Enter does nothing for half a second after it appears, so a key meant for another app can't choose one.

`printf 'red\nblue\n' | dupin pick` opens a transient picker; Enter prints the original value and Esc exits unsuccessfully. For structured values, use `dupin pick --input-json` with a JSON array. Values are displayed as data, including objects containing paths or action names. Picker sessions never add history entries.

`dupin snapshots list --json` prints saved Snapshot metadata, `dupin snapshots open [--id ID]` shows the separate window, and `dupin snapshots close` hides it; see [Snapshots](./snapshots.md#snapshots).

## Configuration

`dupin config get --json` returns the current configuration. Save a copy as `base.json`, edit a second copy, then run `dupin config set base.json desired.json --json`. The base enables conflict detection against external edits. Changes use the settings service, including managed/read-only files and native shortcut validation; a successful reply follows reconciliation. `--json` emits JSON values on success and an `error` object on failure.

`dupin telemetry status`, `dupin telemetry enable` and `dupin telemetry disable` show and change whether the running Dupin sends the [anonymous usage ping](./telemetry.md#opting-out).

`dupin --help`, `dupin --version` and `dupin --licenses` (the third-party notices Dupin.app carries) perform no app or configuration work.

`dupin mcp` serves the same controls, with configuration reading and validation, to agents over the Model Context Protocol; see [MCP server](./mcp.md).

## Headless

These need no resident instance:

```sh
printf '%s' '{"kind":"text","title":"Hello","subtitle":"Headless fixture"}' > /tmp/dupin-view.json
dupin render /tmp/dupin-view.json /tmp/dupin-view.png --json
dupin ext check examples/extensions/word-count-js --json
dupin ext run examples/extensions/word-count-js --query 'two words' --json
```

`dupin ext check DIR` and `dupin ext run DIR --query QUERY` load an extension folder as the launcher does and print its answer; a failure exits non-zero with a diagnostic naming the code, file, field and a fix. `dupin ext test DIR` replays the folder's fixtures and `dupin ext render DIR FIXTURE OUTPUT.png` renders a fixture's panel (see [Testing and rendering](./extensions.md#testing-and-rendering)). They test a folder on its own; the launcher loads [extensions](./extensions.md) from the configuration folder. Headless rendering shows the given text/detail/pair view without executing actions or loading configuration, history or application providers.

## The control socket

Local control uses length-prefixed JSON, one correlated request/reply per connection, a 512 KiB frame cap, at most eight live connections and a five-minute request deadline. The socket lives in an owner-only Dupin directory under `XDG_RUNTIME_DIR` (or the system per-user temporary directory on macOS, independent of shell `TMPDIR`). A live instance is never replaced; an owner-only socket with no listener is recovered on restart. Picker and settings contention return Busy. Disconnecting cancels a pending picker.
