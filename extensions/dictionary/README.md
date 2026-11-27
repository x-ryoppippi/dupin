# Dictionary

Looks a word up in the dictionaries turned on in Dictionary.app and shows the entry.

| Keyword       | `define`, `d` (`dictionary_keywords`)           |
| ------------- | ----------------------------------------------- |
| Command       | `dictionary`                                    |
| On by default | Yes                                             |
| User docs     | [Dictionary](../../docs/features/dictionary.md) |

A port of the former native Dictionary; see [Built-ins as bundled extensions](../../docs/design/extension-platform.md#built-ins-as-bundled-extensions). It answers the built-in `dictionary` command under that command's keywords, so the `keyword` in `package.json` is ignored.

## How it's built

- **`search(query, ctx)`**: looks the trimmed query up and returns one row per entry, up to `ctx.limit`, with the entry's `markdown` as the row's detail and its pronunciation as metadata.
- **Actions:** **Open in Dictionary** opens a `dict://` URL; **Copy Definition** copies the entry as the dictionary wrote it.
- **Errors:** `invalid`, `needs-consent` and `disabled` from the service return no rows, so Dupin shows its own notice or **No definition found**.
- **Capabilities:** `dictionary: ["lookup"]`, to call `dictionary.lookup(term)` while you type. It needs no macOS permission.

## Files

| Path              | What                                                                  |
| ----------------- | --------------------------------------------------------------------- |
| `package.json`    | The manifest.                                                         |
| `main.ts`         | `search`, and the row each entry becomes.                             |
| `fixtures/*.json` | A full entry, a trimmed query, a word with no entry, several entries. |

## Test it

```sh
dupin ext test extensions/dictionary
dupin ext render extensions/dictionary extensions/dictionary/fixtures/01-entry.json entry.png
```

Each fixture answers `dictionary.lookup` from `replies.dictionary`, by term; any other term fails as `unavailable`. Run `dupin` from this checkout's build; see [Working on them](../README.md#working-on-them).
