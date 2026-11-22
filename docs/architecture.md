# Architecture

Dupin is a Cargo workspace of three crates. The UI layer stays thin so the UI framework can change without touching the rest.

| Crate               | Holds                                                                                                                                           |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/dupin-core` | No UI and no macOS: the `Provider` trait and `Item` model, the newest-wins `Searcher`, fuzzy matching, the launcher `Session` and `config.toml` |
| `crates/dupin-mac`  | The apps and Spotlight providers, NSWorkspace actions and the panel's AppKit settings, through objc2                                            |
| `crates/dupin`      | The Rune app: the panel view, key mapping, the hotkey and logging                                                                               |

## Providers

Every source of results is a `Provider`: given a `Request` (query and limit), it streams batches of `Item`s and stops when the request is superseded. Apps, file names and file contents each have one; extensions (scripts that print a JSON view to stdout) are meant to take the same shape.

An `Item` carries its title, subtitle, icon path, the matched characters for highlighting, and two actions: Enter's (`primary`) and ⌘Enter's (`alternate`).

## Searching off the main thread

Each provider runs on its own long-lived thread behind a `Searcher`. The newest request wins: queued requests never start, a running one sees `Request::is_cancelled`, and its late batches are dropped before they reach the UI. A dedicated thread, rather than a pool, lets the Spotlight provider turn that thread's run loop, which `NSMetadataQuery` delivers through.

## The session

`Session` is the launcher's state machine. The UI hands it query edits and key intents, and it answers with effects: start or cancel a search, run an action, clear the field, hide the panel, keep the selection on screen. It owns the mode rules (Space on an empty query enters file search, `in` and Space enter content search, configured dictionary keywords and Space enter Dictionary search, Backspace on an empty query goes back to apps) and drops answers from earlier searches by generation.

Every mode change passes through an empty query, which clears the rows, so the rows on screen always belong to the current mode.

## The panel

The panel is a Rune `launcher_panel`: a non-activating, retained panel that Rune draws ahead of time while hidden, so the hotkey only has to show it. The app is an accessory app (no Dock icon) that starts windowless and keeps running. The panel fits its content height, sits on Liquid Glass (`SystemMaterial::Glass`) under a light frost, and AppKit rounds it and draws its shadow. Behind the panel's content sits a Rune effect slot. A compositor (`backdrop.rs`) draws the backdrop shader into it: the built-in `flow` or a user's WGSL file. `shader.rs` wraps that file in Dupin's prelude and entry point and validates it with naga before it reaches the GPU. The compositor reads the shader's inputs (time since the panel showed, the last keystroke, the mode) from the panel state, and keeps frames coming only while the panel shows.
