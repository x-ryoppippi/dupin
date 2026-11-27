# Web Search

Lists the web searches, such as `g`, `gh`, `maps` and `yt`, that the main search reaches by alias.

| Keyword       | `web`, to list every web search; each search's alias in the main search                     |
| ------------- | ------------------------------------------------------------------------------------------- |
| Command       | `web-search`                                                                                |
| On by default | Yes                                                                                         |
| User docs     | [Web searches](../../docs/features/web-searches.md) and [Maps](../../docs/features/maps.md) |

A port of the former native web searches and Maps; see [Built-ins as bundled extensions](../../docs/design/extension-platform.md#built-ins-as-bundled-extensions).

## How it's built

- **`items(ctx)`**: one catalogue item per web search, each with a `webSearch` of its alias, URL template and title. Dupin matches the aliases and fills `{query}` into the URL natively, so typing never waits on the guest.
- **`defaultSearch: true`**: the catalogue is listed in the main search too, beside apps.
- **Preferences:** none declared. Dupin hands the shipped package `webSearches` and `webFallbacks` from config.toml's `web_searches` and `web_fallbacks`, once checked; without `webSearches`, the package lists its four defaults.
- **Capabilities:** none.

## Files

| Path              | What                                                                  |
| ----------------- | --------------------------------------------------------------------- |
| `package.json`    | The manifest.                                                         |
| `main.ts`         | The defaults and `items`.                                             |
| `fixtures/*.json` | The defaults, searches from config.toml, and every search turned off. |

## Test it

```sh
dupin ext test extensions/web-search
dupin ext render extensions/web-search extensions/web-search/fixtures/defaults.json defaults.png
```

`configured.json` and `disabled.json` set config.toml's searches in the fixture's `preferences`, as Dupin hands them over. Run `dupin` from this checkout's build; see [Working on them](../README.md#working-on-them).
