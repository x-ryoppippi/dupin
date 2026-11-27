# Bundled extensions

Each folder here is an extension package that ships inside Dupin.app. They run in the same sandboxed extension host, under the same limits, as any package you install, so each one is a working example to copy. [Bundled extensions](../docs/features/extensions.md#bundled-extensions) covers how they ship, their grants and how to override one.

| Package                                   | Keyword               | On by default              | What it does                                                     |
| ----------------------------------------- | --------------------- | -------------------------- | ---------------------------------------------------------------- |
| [agent](./agent/)                         | `agent`               | No                         | Lists running coding agents and the project each works in.       |
| [calculator](./calculator/)               | None: the main search | Yes (`calculator`)         | Answers calculations and unit and currency conversions.          |
| [calendar](./calendar/)                   | `cal`                 | No (`[calendar] enabled`)  | Lists and searches calendar events.                              |
| [clipboard-history](./clipboard-history/) | `clip`                | No (`[clipboard] enabled`) | Keeps what you copy and copies or pastes it again.               |
| [dictionary](./dictionary/)               | `define`, `d`         | Yes                        | Looks words up in Dictionary.app's dictionaries.                 |
| [dupin](./dupin/)                         | `help`                | Yes                        | Links to Dupin's documentation, issues, source and website.      |
| [emoji](./emoji/)                         | `emoji`               | Yes                        | Picks an emoji or symbol by its English or Japanese name.        |
| [port](./port/)                           | `port`                | No                         | Lists listening ports and stops the process holding one.         |
| [proc](./proc/)                           | `proc`                | No                         | Lists processes and stops one.                                   |
| [repo](./repo/)                           | `repo`                | No                         | Lists Git repositories and opens one in an editor or terminal.   |
| [snippets](./snippets/)                   | `snip`                | Yes                        | Copies or pastes saved text, and adds and edits stored snippets. |
| [web-search](./web-search/)               | `web`, and each alias | Yes                        | Lists the web searches the main search reaches by alias.         |
| [window-management](./window-management/) | `window`              | No                         | Moves and resizes the focused window.                            |

A package that is off by default waits for its switch in Settings → Extensions. A setting in brackets is a former built-in's own switch, which must be on too; calculator, calendar, clipboard-history, dictionary, snippets, web-search and window-management are ports of former built-ins.

## Where to start

| To learn                                               | Read                                                                                                      |
| ------------------------------------------------------ | --------------------------------------------------------------------------------------------------------- |
| `search(query)` and nothing else                       | [dupin](./dupin/), then [dictionary](./dictionary/) for a host service and a detail pane.                 |
| `search` with sections and `emptyQuery`                | [calendar](./calendar/).                                                                                  |
| An `items()` catalogue, sent in parts                  | [emoji](./emoji/).                                                                                        |
| Web search items and config.toml preferences           | [web-search](./web-search/).                                                                              |
| The main search (`defaultSearch`) and `run` actions    | [window-management](./window-management/).                                                                |
| Pushed views and forms                                 | [snippets](./snippets/) for a form, [proc](./proc/) for a detail page.                                    |
| `events.clipboard`, `storage` and `items.invalidate()` | [clipboard-history](./clipboard-history/).                                                                |
| `exec` profiles and a catalogue with a `ttl`           | [port](./port/) is the smallest; [proc](./proc/) and [agent](./agent/) add more profiles.                 |
| `files` scopes, declared preferences and stored state  | [repo](./repo/).                                                                                          |
| Filters and kind actions (`dupin.provides`)            | No bundled package provides any yet; see [Advanced composition](../docs/features/advanced.md#extensions). |

[`examples/extensions`](../examples/extensions) holds two smaller packages that don't ship: a word counter and a notes list.

## Extension docs

- [Manifest](../docs/features/extension-manifest.md): the `dupin` key of `package.json`.
- [API](../docs/features/extension-api.md): `search`, `items`, actions, views, forms and events.
- [Capabilities](../docs/features/extension-permissions.md): what a package may ask to use.
- [Developing and testing](../docs/features/extension-testing.md): `dupin ext` and fixtures.
- [Distributing](../docs/features/extension-distribution.md): building, sharing and installing.
- [Built-ins as bundled extensions](../docs/design/extension-platform.md#built-ins-as-bundled-extensions): why each former built-in became a package.

## Working on them

Every package is TypeScript that Dupin runs as written, typed by [`sdk/dupin.d.ts`](../sdk/dupin.d.ts). In `nix develop`:

```sh
tsc --project extensions/tsconfig.json     # type-check every package
cargo run -p dupin -- ext test extensions/calendar
cargo run -p dupin -- ext render extensions/calendar extensions/calendar/fixtures/upcoming.json upcoming.png
```

Run `dupin` from this checkout's build, as above, so a package here replays as the bundled package it is; the READMEs write it as plain `dupin`. `cargo test -p dupin every_bundled_extension_passes_its_fixtures` replays every package's fixtures, and the `extensions::overrides` test fails for a package added here without a case of its own.

The build seals each package folder by a hash of every file in it, its README and fixtures included, and ships the folder whole; a folder that no longer matches stays off, so rebuild after editing one. A package's revision, which its grants follow, hashes only `package.json` and its modules.
