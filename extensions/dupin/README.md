# Dupin

Links to Dupin's documentation, issues, source code and website.

| Keyword       | `help`                                                                     |
| ------------- | -------------------------------------------------------------------------- |
| Command       | `dupin`                                                                    |
| On by default | Yes                                                                        |
| User docs     | [Bundled extensions](../../docs/features/extensions.md#bundled-extensions) |

## How it's built

The smallest bundled package, and the one to copy for a first extension.

- **`search(query)`** only: it filters six static rows, each with a `url`, so <kbd>↩</kbd> opens the page. A row matches when every typed word is in its title or subtitle; nothing is listed before a word is typed.
- **Capabilities:** none.

## Files

| Path              | What                                                                                                       |
| ----------------- | ---------------------------------------------------------------------------------------------------------- |
| `package.json`    | The manifest.                                                                                              |
| `main.ts`         | `search` and its rows.                                                                                     |
| `fixtures/*.json` | `help.json`: nothing before a word, and <kbd>↩</kbd> opens a row's page. `search.json`: which words match. |

## Test it

```sh
dupin ext test extensions/dupin
dupin ext render extensions/dupin extensions/dupin/fixtures/search.json search.png
```

The fixtures need no `replies`, since the package asks the host for nothing: each types queries and checks the rows, and `help.json` activates rows to check their URLs. Run `dupin` from this checkout's build; see [Working on them](../README.md#working-on-them).
