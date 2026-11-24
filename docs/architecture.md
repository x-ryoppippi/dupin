# Architecture

The search and ranking model must remain independent of UI frameworks and operating-system APIs. Native providers own the macOS boundary; the thin UI layer adapts input and renders results so that the UI framework can change without replacing the model.

## Providers

Every source of results is a `Provider` that streams batches of `Item`s and stops when its request is superseded. A `Request` carries the query, candidate budget and an optional `page` for a pushed view. Apps, file names and file contents each have a provider. Each loaded [extension](./features/extensions.md) has a provider that runs its guest and turns the returned views and actions into items. The folder's manifest picks the runtime: for a `package.json` with a `dupin` key, `dupin-js-runtime` runs the JavaScript package on QuickJS-ng, in a fresh runtime per query with its own heap, stack, deadline and promise-job limits, and for an `extension.toml`, `dupin-plugin-runtime` runs the Wasm module it names. Both answer in `dupin-plugin-protocol` rows, so copy, URL and push actions reach the session the same way.

An `Item` owns the runtime-independent `dupin-plugin-protocol::View`: Text (title/subtitle), Detail (title/body), or Pair (source/result). It also carries its stable ID, icon path, matched title characters, optional score, action list and primary/alternate action IDs. `display_title()` returns the Text/Detail title or Pair source. Explicit `record_usage` metadata controls history recording; apps/files opt in and dictionary results opt out, independently of view.

A provider that groups or lays out its rows overrides `Provider::search_batches`, which hands over a `Batch`: rows, an optional section title, and a `Detail` for any row that should show as a page. A `Detail` has a title, labeled metadata and a Markdown body that `dupin-core` parses once, with pulldown-cmark, into a bounded document of headings, paragraphs, lists and code blocks with bold, italic, code and link runs. Input past 32 KiB, blocks past 512 and lists nested past three levels are cut or flattened. Only http and https links are pressable; raw HTML and other schemes stay inert text. The session keeps sections and details beside the rows, by origin and item ID, so ranking, selection restoration and back navigation treat a sectioned row like any other. The row's own `View` stays the plain-text form for the CLI and snapshots.

The host renders each item in declared order. All-text lists retain fixed-height virtualisation; a sectioned list adds a heading row where the section changes and lays out every row, and mixed results use a bounded scrolling column with selectable detail/pair cards and stable row/action anchors. Text rows keep an icon column only when some row in the list has an icon, so an icon-less list such as calendar events or snippets starts its titles at the row's edge while a mixed list keeps them aligned. Keyboard selection moves between rows only; a heading shares its first row's key, so revealing that row shows the heading too. A pressed link goes back to the session, which opens it only if the current row's detail links to it.

Registered routes and shared internal views advance the plugin migration. Native providers still call OS APIs directly. `dupin-broker` holds the extension authority model, with declared capabilities, per-revision grants and revocable permits ([capability broker](./capability-broker.md)), but no host service goes through it yet. User extensions get no host operations at all, so they need neither.

Extension authoring runs outside the launcher. `dupin-ext-sdk` backs `dupin ext new`, `build` and `dev`: it scaffolds packages with the declarations in `sdk/dupin.d.ts`, bundles TypeScript with the author's own Bun or esbuild, and publishes development revisions behind a symlink the extensions scan already follows. It checks packages with `dupin-js-runtime`, as the launcher loads them. Dupin never bundles, downloads a bundler or type-checks at run time; extension providers only load the JavaScript these commands produce.

## Searching off the main thread

Trusted [script commands](./features/scripts.md) use a separate native execution path. Discovery only reads bounded headers; activation runs the executable directly on a worker with bounded output and an owned process group. The lifecycle lock serializes spawning and group registration with synchronous quit cleanup. Script operations are absent from the guest protocol.

Each provider runs on its own long-lived thread behind a `Searcher`. The newest request wins: queued requests never start, a running one sees `Request::is_cancelled`, and its late batches are dropped before they reach the UI. A dedicated thread, rather than a pool, lets the Spotlight provider turn that thread's run loop, which `NSMetadataQuery` delivers through.

Candidate budgets and display limits are separate. The worker enforces each request's cumulative output budget; the session stores at most that many unique candidates, applies usage ranking, then cuts each provider and the merged list to their display limits. A chosen row or open action menu keeps its target in a visible slot while ranking changes, except when the only available slot belongs to the pinned provider. Automatic first-row selection does not pin a weak result indefinitely.

## The session

`Session` is the launcher's state machine. The host registers commands and their routes. The UI hands the active session query edits and key intents. The session answers with effects such as starting or cancelling a search, running an action, clearing the field, hiding the panel or keeping the selection on screen. It applies registered keyword and empty-query routes and drops answers from earlier searches by generation.

`Action::Push` opens a view of the selected item's command with a fresh query. Browsing a folder uses the same navigation stack and searches the browser command. Esc closes an extension view and restores its parent query and selection. Backspace (⌫) returns to the previous view or folder when the query is empty. At the base of the stack, Backspace on an empty query returns to the registered default command.

Switching commands clears the previous rows. Generation checks reject late answers from the old command.

## Configuration changes

The resident panel owns the active configuration and hotkey binding; the Settings window reflects accepted values. One worker orders settings writes and watches the configured path and its symlink targets through the shared FSEvents watcher. Reloaded values use the same panel apply path as Settings and are never written back merely because they were read.

Each queued save has a revision. Reads carry the last revision the worker processed, and both views reject a snapshot older than the newest submitted save. Settings queues its panel update before submitting a save, so the worker's own file event cannot register the same hotkey twice. Parse and hotkey errors preserve the active values and remain visible without opening a window.

## Stored data

Data that must survive a restart without sitting on disk in the clear goes through `dupin-storage`, which has no OS dependencies. It gives each owner a namespace of sealed JSON records and content-addressed blobs with its own retention limits, and takes its data key from a `KeyProvider`: `dupin-mac::keychain::KeychainKey` on macOS, a private key file elsewhere, an in-memory key in tests. The host owns the files and the key; an extension will reach its own namespace only through the broker's `storage` capability. See [Encrypted storage](./features/storage.md).

## The panel

The panel is a Rune `launcher_panel`: a non-activating, retained panel that Rune draws ahead of time while hidden, so the hotkey only has to show it. The app is an accessory app (no Dock icon) that starts windowless and keeps running. The panel fits its content height, sits on Liquid Glass (`SystemMaterial::Glass`) under a light frost, and AppKit rounds it and draws its shadow. Behind the panel's content sits a Rune effect slot. A compositor (`backdrop.rs`) draws the backdrop shader into it: the built-in `flow` or a user's WGSL file. `shader.rs` wraps that file in Dupin's prelude and entry point and validates it with naga before it reaches the GPU. The compositor reads the shader's inputs (time since the panel showed, the last keystroke, the mode) from the panel state, and keeps frames coming only while the panel shows.
