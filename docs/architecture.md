# Architecture

The search and ranking model must remain independent of UI frameworks and operating-system APIs. Native providers own the macOS boundary; the thin UI layer adapts input and renders results so that the UI framework can change without replacing the model.

## Providers

Every source of results is a `Provider`: given a `Request` (query and candidate budget), it streams batches of `Item`s and stops when the request is superseded. Apps, file names and file contents each have one; extensions (scripts that print a JSON view to stdout) are meant to take the same shape.

An `Item` owns the runtime-independent `dupin-plugin-protocol::View`: Text (title/subtitle), Detail (title/body), or Pair (source/result). It also carries its stable ID, icon path, matched title characters, optional score, action list and primary/alternate action IDs. `display_title()` returns the Text/Detail title or Pair source. Explicit `record_usage` metadata controls history recording; apps/files opt in and dictionary results opt out, independently of view.

The host renders each item in declared order. All-text lists retain fixed-height virtualisation; mixed results use a bounded scrolling column with selectable detail/pair cards and stable row/action anchors. Content is plain text.

Registered routes and shared internal views advance the plugin migration. Native providers still call OS APIs directly: the native OS broker, external manifests/packages/runtime integration and plugin grants remain incomplete. The protocol dependency adds no plugin execution runtime to the app.

## Searching off the main thread

Trusted [script commands](./scripts.md) use a separate native execution path. Discovery only reads bounded headers; activation runs the executable directly on a worker with bounded output and an owned process group. The lifecycle lock serializes spawning and group registration with synchronous quit cleanup. Script operations are absent from the guest protocol.

Each provider runs on its own long-lived thread behind a `Searcher`. The newest request wins: queued requests never start, a running one sees `Request::is_cancelled`, and its late batches are dropped before they reach the UI. A dedicated thread, rather than a pool, lets the Spotlight provider turn that thread's run loop, which `NSMetadataQuery` delivers through.

Candidate budgets and display limits are separate. The worker enforces each request's cumulative output budget; the session stores at most that many unique candidates, applies usage ranking, then cuts each provider and the merged list to their display limits. A chosen row or open action menu keeps its target in a visible slot while ranking changes, except when the only available slot belongs to the pinned provider. Automatic first-row selection does not pin a weak result indefinitely.

## The session

`Session` is the launcher's state machine. The host registers commands and their routes; the UI hands the active session query edits and key intents, and it answers with effects: start or cancel a search, run an action, clear the field, hide the panel, keep the selection on screen. It applies registered keyword and empty-query routes, returns to the registered default on Backspace, and drops answers from earlier searches by generation. Browsing a folder pushes the current command, query and selection onto a stack and searches the registered browser command with that folder; Backspace on an empty query pops it.

Switching commands clears the previous rows. Generation checks reject late answers from the old command.

## Configuration changes

The resident panel owns the active configuration and hotkey binding; the Settings window reflects accepted values. One worker orders settings writes and watches the configured path and its symlink targets through the shared FSEvents watcher. Reloaded values use the same panel apply path as Settings and are never written back merely because they were read.

Each queued save has a revision. Reads carry the last revision the worker processed, and both views reject a snapshot older than the newest submitted save. Settings queues its panel update before submitting a save, so the worker's own file event cannot register the same hotkey twice. Parse and hotkey errors preserve the active values and remain visible without opening a window.

## The panel

The panel is a Rune `launcher_panel`: a non-activating, retained panel that Rune draws ahead of time while hidden, so the hotkey only has to show it. The app is an accessory app (no Dock icon) that starts windowless and keeps running. The panel fits its content height, sits on Liquid Glass (`SystemMaterial::Glass`) under a light frost, and AppKit rounds it and draws its shadow. Behind the panel's content sits a Rune effect slot. A compositor (`backdrop.rs`) draws the backdrop shader into it: the built-in `flow` or a user's WGSL file. `shader.rs` wraps that file in Dupin's prelude and entry point and validates it with naga before it reaches the GPU. The compositor reads the shader's inputs (time since the panel showed, the last keystroke, the mode) from the panel state, and keeps frames coming only while the panel shows.
