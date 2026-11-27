# Clipboard history

Keeps the text, images and files you copy, encrypted, and copies or pastes one again.

| Keyword       | `clip`                                                         |
| ------------- | -------------------------------------------------------------- |
| Command       | `clipboard`                                                    |
| On by default | No: it waits for **Clipboard history** (`[clipboard] enabled`) |
| User docs     | [Clipboard history](../../docs/features/clipboard.md)          |

A port of the former native clipboard history; see [Built-ins as bundled extensions](../../docs/design/extension-platform.md#built-ins-as-bundled-extensions). It answers the built-in command under that command's keyword, and Dupin delivers copies only while that setting is on.

## How it's built

The bundled example of a catalogue kept up to date by an event handler.

- **`events.clipboard(change)`**: stores each copy in the package's `storage`, keyed by its content so a copy made again moves to the top, with an image's PNG as a blob that never crosses to the guest. It trims entries past the retention and count preferences, then calls `items.invalidate()`.
- **`items(ctx)`**: lists the stored copies newest first, in parts (`more` and a cursor), with a `ttl`. On `reason: "invalidated"` it answers an `update` that prepends new copies and removes the ones gone, rather than listing everything again.
- **`match: "words"`**: Dupin matches the catalogue as a history is searched, every typed word in any order.
- **Actions:** **Copy** and **Paste** (`run` handlers that put the copy back with `clipboard.write` and return `hide` or `paste`), **Delete from History** and **Clear History**, which asks to `confirm` first.
- **Preferences:** `retention`, `includeImages`, `includeFiles` and `maxEntries`, which Dupin fills from the `[clipboard]` Settings rows.
- **Capabilities:**
  - `clipboard: ["watch", "write", "paste"]`: `watch` for `events.clipboard`, `write` to put a copy back, `paste` for the Paste action.
  - `storage: { quotaBytes: 67108864 }`: 64 MiB of encrypted storage for the history and its images.

## Files

| Path              | What                                                                                                |
| ----------------- | --------------------------------------------------------------------------------------------------- |
| `package.json`    | The manifest and its preferences.                                                                   |
| `main.ts`         | The clipboard handler, the catalogue and its updates, and the actions.                              |
| `fixtures/*.json` | `capture.json`: copies arriving as events. `history.json`: a seeded history listed and copied back. |

## Test it

```sh
dupin ext test extensions/clipboard-history
dupin ext render extensions/clipboard-history extensions/clipboard-history/fixtures/history.json history.png
```

`capture.json` delivers copies with `{ "event": "clipboard", "data": … }` steps and checks the `storage` and `invalidated` each produced. `history.json` seeds the store with `replies.storage`. Run `dupin` from this checkout's build; see [Working on them](../README.md#working-on-them).
