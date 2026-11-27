# Emoji & Symbols

Picks an emoji or symbol by its English or Japanese name, and copies or pastes it.

| Keyword       | `emoji`                                         |
| ------------- | ----------------------------------------------- |
| Command       | `emoji`                                         |
| On by default | Yes                                             |
| User docs     | [Emoji & Symbols](../../docs/features/emoji.md) |

## How it's built

The bundled example of a large static catalogue.

- **`items(ctx)`**: turns the rows of `catalogue.ts` into catalogue items 500 at a time, answering `more` and a cursor until every row is sent, each part after the first as an `update` that appends. Dupin holds the catalogue and matches each keystroke against it, so the guest isn't called while you type.
- Each item has a `section` (its Unicode group), `keywords` (its name and its English and Japanese keywords) and `detail` metadata: the character, its names, its code points and its keywords.
- **Actions:** **Copy** and **Paste** as plain `copy` and `paste` actions, and one Copy per skin tone where the emoji has them.
- **Capabilities:** `clipboard: ["paste"]`, for the Paste action. Copying needs no capability.

## Files

| Path                  | What                                                                                                              |
| --------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `package.json`        | The manifest.                                                                                                     |
| `main.ts`             | `items`, and the item each row becomes.                                                                           |
| `catalogue.ts`        | Generated: one tab-separated row per character, from Unicode Emoji 15.1 and CLDR 45. Don't edit it by hand.       |
| `scripts/generate.nu` | Writes `catalogue.ts`: `nu extensions/emoji/scripts/generate.nu`, with `--emoji` and `--cldr` for other releases. |
| `LICENSE-UNICODE`     | The Unicode License v3, which covers the data.                                                                    |
| `fixtures/*.json`     | Browsing the groups, English and Japanese names, symbols, copying, pasting and a skin tone.                       |

## Test it

```sh
dupin ext test extensions/emoji
dupin ext render extensions/emoji extensions/emoji/fixtures/browse.json browse.png
```

The fixtures need no `replies`: each types a query and checks the catalogue's matches and their actions. Run `dupin` from this checkout's build; see [Working on them](../README.md#working-on-them).
