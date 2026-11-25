# Advanced composition

Every built-in search is a composition: sources produce items, filters match and order them, and kinds give each row its actions. The features supply the whole composition, so `config.toml` needs no `[advanced]` table, and without one Dupin behaves exactly as it does with an empty file. `[advanced]` changes parts of that composition and leaves the rest as the features set it.

```toml
[advanced.sources.apps]
matcher = "builtin.prefix"

[advanced.commands.default]
append_sources = ["files"]

[[advanced.kinds.file.actions]]
title = "Open in Zed"
script = "open-in-zed"

[[advanced.kinds.url.actions]]
title = "Open in the Wayback Machine"
url = "https://web.archive.org/web/{url}"
```

Settings → Search lists the composition `[advanced]` sets, read-only; edit `config.toml` to change it. With the [Home Manager module](../configuration.md#nix), write it under `settings.advanced`, which locks the whole table in Settings.

## Sources

The native sources are `apps` (installed apps, the main search), `files` (Spotlight file search), `files.contents` (Spotlight content search, `in`), `folder` ([folder browsing](./folder-browsing.md)) and `recent` ([recent items](./recent-items.md)).

`[advanced.sources.<source>]` sets a source's filter stages. Only `apps` and `folder` match their own candidates; Spotlight and the recent items list match through indexes of their own and take neither stage.

| Key       | Values                                                                                                                                                                                                                                                                        |
| --------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `matcher` | `builtin.fuzzy` (the default) finds the query's letters in order, ranking prefixes, word starts and runs first. `builtin.prefix` finds only names, or words in them, that start with the query, ignoring case.                                                                |
| `sorter`  | `builtin.relevance` (the default) shows the best matches first, and usage history moves what you pick up. `builtin.title` orders the matches by name, ignoring case, and usage history leaves that order alone. A browsed folder keeps its folders first under either sorter. |

## The main search

`[advanced.commands.default]` `append_sources` lists sources the main search also searches, after its own. `files` and `files.contents` add Spotlight results, up to `file_limit` of them, below the apps. `apps` is the main search's own source and `folder` lists only a folder being browsed, so neither can be appended. A source whose feature is off, such as `recent` with `show_recent = false`, is left out with a notice. Extensions aren't sources here: a package's catalogue joins the main search only when its manifest asks to (`defaultSearch`).

## Kind actions

`[[advanced.kinds.<kind>.actions]]` adds an action to every row of a kind, after the kind's own actions, in the action screen (⌘K). Enter, ⌘Enter and Send to… are unchanged. The kinds are `text`, `url`, `file` (files and folders) and `app`. A kind takes up to 16 actions, each with a `title` and exactly one of:

- `script`: the file name of a [script command](./scripts.md) in the scripts folder. Dupin finds the script when the action runs and passes the row's value as its one argument, so the script declares one, such as `# @dupin.argument path`. The script's output mode decides whether the panel stays open.
- `url`: an `http` or `https` URL with a host, opened with the row's value percent-encoded into its placeholders, which sit after the host. `text` rows fill `{text}`, `url` rows `{url}`, and `file` and `app` rows `{path}` and `{name}` (the file name).

The value is the text, the URL, or the file's or app's path. A path that isn't valid UTF-8 gets no configured actions, since it would reach the script or URL as a different path.

`file` and `app` actions appear on rows from Dupin's own sources: app search, file and content search, folder browsing and recent items. An [extension](./extensions.md)'s row is text where it copies text as written and a URL where it opens one, so the first text it copies and the first URL it opens take the `text` and `url` actions, which only you can run; the extension never learns they exist. A path an extension names (`file`) is one Dupin hasn't checked the extension may act on, so it takes no `file` or `app` actions.

## Problems

A key Dupin doesn't read, a source, matcher or sorter it doesn't have, a stage a source doesn't take, a script name with a `/`, or a URL template outside these rules makes `config.toml` invalid, with the problem named. At launch Dupin then uses the defaults; during a reload it keeps the last valid configuration, its composition included. A missing script is reported when its action runs.
