# Advanced composition

`[advanced]` in `config.toml` changes how Dupin's searches are put together: which matcher and sorter a source uses, what the main search also searches, and which actions every row of a kind gets, in what order. This composition follows the model of Shougo Matsushita's [ddu.vim](https://github.com/Shougo/ddu.vim).

## Open it

Edit the `[advanced]` table in `config.toml`; Settings → Search shows what it sets.

## What you see

- In Settings → Search, the composition `[advanced]` sets, read-only.
- Rows matched and ordered by the matcher and sorter you chose.
- In the main search, results from the sources you appended, below the apps.
- In the action screen, the actions you added to a kind, after the kind's own.
- In the action screen, a kind's actions in the order you set, without the ones you hid.

## What you can do

| Key / action                                                         | What it does                                                                                        |
| -------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| <kbd>→</kbd> at the end of the input, <kbd>⌘K</kbd> or <kbd>⌃K</kbd> | Opens the action screen, which lists the actions you added to the row's kind, in the order you set. |
| A `script` action                                                    | Runs the [script command](./scripts.md) with the row's value as its one argument.                   |
| A `url` action                                                       | Opens the URL with the row's value filled in.                                                       |
| An `extension` action                                                | Runs the extension's kind action on the row's value, once you allow it.                             |
| <kbd>↩</kbd>, <kbd>⌘↩</kbd>, <kbd>⌘Y</kbd>, **Send to…**             | Unchanged: they do what the row's kind always does, even when you hid the action they run.          |

Other keys work as everywhere in the panel; see [Keys](../usage.md#keys).

## Notes

- A table changes only the parts it names; without one, Dupin behaves as it does with an empty file.
- Hiding an action only takes it out of the action screen; see [Hide and order](#hide-and-order).
- An invalid `[advanced]` makes `config.toml` invalid; see [Problems](#problems).
- With the [Home Manager module](../configuration.md#nix), write it under `settings.advanced`, which locks the whole table in Settings.

<details>
<summary>More notes</summary>

- Only `apps` and `folder` take a matcher or sorter.
- An extension's own rows never get `file` or `app` actions, and no extension learns about the `text` and `url` actions you add to its rows.

</details>

## Configuration

```toml
[advanced.sources.apps]
matcher = "builtin.prefix"

[[advanced.kinds.url.actions]]
title = "Open in the Wayback Machine"
url = "https://web.archive.org/web/{url}"

[advanced.kinds.file]
hide = ["quick-look"]
order = ["reveal"]
```

| Table                               | Changes                                                                               |
| ----------------------------------- | ------------------------------------------------------------------------------------- |
| `[advanced.sources.<source>]`       | A source's [matcher and sorter](#sources).                                            |
| `[advanced.commands.default]`       | What [the main search](#the-main-search) searches besides apps.                       |
| `[[advanced.kinds.<kind>.actions]]` | [Actions](#kind-actions) added to every row of a kind.                                |
| `[advanced.kinds.<kind>]`           | Which of a kind's actions the action screen [hides and lists first](#hide-and-order). |

<details>
<summary>How composition works, and a fuller example</summary>

Every built-in search is a composition: sources produce items, filters match and order them, and kinds give each row its actions. `[advanced]` changes parts of that composition and leaves the rest as the features set it. Settings → Search lists the composition `[advanced]` sets, read-only; edit `config.toml` to change it.

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

</details>

## Sources

`[advanced.sources.<source>]` sets the `matcher` and `sorter` of `apps` (installed apps) or `folder` ([folder browsing](./folder-browsing.md)).

| Key       | Default             | Values                                                                                   |
| --------- | ------------------- | ---------------------------------------------------------------------------------------- |
| `matcher` | `builtin.fuzzy`     | `builtin.fuzzy`, `builtin.prefix`, or a [package's](#extensions) `<package>/<filter>`    |
| `sorter`  | `builtin.relevance` | `builtin.relevance`, `builtin.title`, or a [package's](#extensions) `<package>/<filter>` |

<details>
<summary>Every source, and what the built-ins do</summary>

| Source           | Is                                      | Takes filter stages |
| ---------------- | --------------------------------------- | ------------------- |
| `apps`           | Installed apps, the main search         | Yes                 |
| `files`          | Spotlight file search                   | No                  |
| `files.contents` | Spotlight content search, `in`          | No                  |
| `folder`         | [Folder browsing](./folder-browsing.md) | Yes                 |
| `recent`         | [Recent items](./recent-items.md)       | No                  |

Only `apps` and `folder` match their own candidates; Spotlight and the recent items list match through indexes of their own and take neither stage.

| Built-in            | Does                                                                                                                                                                            |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `builtin.fuzzy`     | Finds the query's letters in order, ranking prefixes, word starts and runs first, and a name or word the query starts above every other match, however often that one was used. |
| `builtin.prefix`    | Finds only names, or words in them, that start with the query, ignoring case.                                                                                                   |
| `builtin.relevance` | Shows the best matches first, and usage history moves what you pick up.                                                                                                         |
| `builtin.title`     | Orders the matches by name, ignoring case, and usage history leaves that order alone.                                                                                           |

A browsed folder keeps its folders first under either sorter.

</details>

## The main search

`[advanced.commands.default]` `append_sources` lists sources the main search also searches, after its own: `files` and `files.contents` add Spotlight results, up to `file_limit` of them, below the apps.

<details>
<summary>What can't be appended</summary>

`apps` is the main search's own source and `folder` lists only a folder being browsed, so neither can be appended. A source whose feature is off, such as `recent` with `show_recent = false`, is left out with a notice. Extensions aren't sources here: a package's catalogue joins the main search only when its manifest asks to (`defaultSearch`).

</details>

## Kind actions

`[[advanced.kinds.<kind>.actions]]` adds up to 16 actions to every row of a kind (`text`, `url`, `file` or `app`), after the kind's own, in the action screen. Each has a `title` and one of `script`, `url` or `extension`.

<details>
<summary>Kinds, keys and placeholders</summary>

Enter, ⌘Enter and Send to… are unchanged.

| Kind   | Rows              | Value passed    | URL placeholders   |
| ------ | ----------------- | --------------- | ------------------ |
| `text` | Text              | The text        | `{text}`           |
| `url`  | URLs              | The URL         | `{url}`            |
| `file` | Files and folders | The file's path | `{path}`, `{name}` |
| `app`  | Apps              | The app's path  | `{path}`, `{name}` |

| Key         | Runs                                                                                                                                                                             |
| ----------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `script`    | The file name of a [script command](./scripts.md) in the scripts folder, with the row's value as its one argument, so the script declares one, such as `# @dupin.argument path`. |
| `url`       | An `http` or `https` URL with a host, opened with the row's value percent-encoded into its placeholders, which sit after the host. `{name}` is the file name.                    |
| `extension` | An [extension's](#extensions) kind action, `<package>/<name>`.                                                                                                                   |

</details>

<details>
<summary>Which rows get them</summary>

Dupin finds the script when the action runs. The script's output mode decides whether the panel stays open. A path that isn't valid UTF-8 gets no configured actions, since it would reach the script or URL as a different path.

`file` and `app` actions appear on rows from Dupin's own sources: app search, file and content search, folder browsing and recent items. An [extension](./extensions.md)'s row is text where it copies text as written and a URL where it opens one, so the first text it copies and the first URL it opens take the `text` and `url` actions, which only you can run; the extension never learns they exist. A path an extension names (`file`) is one Dupin hasn't checked the extension may act on, so it takes no `file` or `app` actions.

</details>

## Hide and order

`[advanced.kinds.file]` and `[advanced.kinds.app]` take `hide` and `order`, lists of action IDs. `hide` leaves actions out of the action screen. `order` lists actions first, in its order; every other action follows in its usual place.

```toml
[advanced.kinds.file]
hide = ["quick-look", "compress"]
order = ["reveal", "open"]
```

| Key     | Takes                                                     | Does                                                                           |
| ------- | --------------------------------------------------------- | ------------------------------------------------------------------------------ |
| `hide`  | Action IDs of the kind, or `<package>/<action>`, up to 32 | Leaves them out of the action screen. Their keys still run them.               |
| `order` | The same                                                  | Lists them first, in this order, then the rest as usual, Send to… always last. |

<details>
<summary>Every action ID</summary>

The IDs are stable: they name the same action in every release.

| Kind   | ID           | Action                                     |
| ------ | ------------ | ------------------------------------------ |
| `file` | `open`       | Open, which <kbd>↩</kbd> runs              |
| `file` | `browse`     | Browse, on folders only                    |
| `file` | `open-with`  | Open with…                                 |
| `file` | `reveal`     | Reveal in Finder, which <kbd>⌘↩</kbd> runs |
| `file` | `quick-look` | Quick Look, which <kbd>⌘Y</kbd> runs       |
| `file` | `copy-name`  | Copy name                                  |
| `file` | `rename`     | Rename…                                    |
| `file` | `move-to`    | Move to…                                   |
| `file` | `copy-to`    | Copy to…                                   |
| `file` | `duplicate`  | Duplicate                                  |
| `file` | `compress`   | Compress                                   |
| `file` | `trash`      | Move to Trash                              |
| `app`  | `open`       | Open, which <kbd>↩</kbd> runs              |
| `app`  | `reveal`     | Reveal in Finder, which <kbd>⌘↩</kbd> runs |
| `app`  | `quick-look` | Quick Look, which <kbd>⌘Y</kbd> runs       |

A file row lists them in this order by default. `rename` to `compress` appear only on a name that is valid UTF-8, and `browse` only on a folder; naming one a row lacks changes nothing on that row.

</details>

<details>
<summary>Keys, composed actions and the other kinds</summary>

Hiding an action never turns it off. <kbd>↩</kbd> and <kbd>⌘↩</kbd> still run `open` and `reveal`, and <kbd>⌘Y</kbd> still runs `quick-look`, so you can keep a key you use without its row. A hidden action that asks you to confirm first is shown for that prompt only.

A [package's kind action](#extensions) is named `<package>/<action>`, as `extension` names it; one no package composes on the kind changes nothing. A `script` or `url` action has no ID: it keeps its place among the actions you added, after any you list in `order`. Remove it from `actions` to hide it.

`text` and `url` take neither key: an extension's row lists the extension's own actions, and Dupin's only `text` and `url` actions are the ones <kbd>↩</kbd> runs.

</details>

## Extensions

An extension can provide filters and kind actions, declared in the `provides` object of its [manifest](./extension-manifest.md). Nothing is composed until `config.toml` names one as `<package>/<name>`:

```toml
[advanced.sources.apps]
matcher = "kana/romaji"
```

<details>
<summary>Declaring and exporting them</summary>

`<package>` is the package's `name` in its package.json, and each is exported from the package's entry module.

```toml
[advanced.sources.apps]
matcher = "kana/romaji"

[[advanced.kinds.file.actions]]
title = "Upload"
extension = "uploader/upload"
```

```json
"dupin": {
	"title": "Kana",
	"keyword": "kana",
	"capabilities": { "rows": { "app": ["title"] }, "query": ["read"] },
	"provides": { "filters": { "romaji": { "stage": "matcher" } } }
}
```

```js
export const filters = {
	romaji(input) {
		return input.candidates
			.map((candidate, index) => ({ index, score: 0, title: toRomaji(candidate.title) }))
			.filter((found) => found.title.startsWith(input.query))
			.map(({ index, score }) => ({ index, score }));
	},
};
```

| Provides                       | Declared with                            | Composed as                                       | Needs                                                         |
| ------------------------------ | ---------------------------------------- | ------------------------------------------------- | ------------------------------------------------------------- |
| `provides.filters.<name>`      | `stage`: `matcher` or `sorter`           | `matcher` or `sorter` of `apps` or `folder`       | `rows:app` or `rows:file` with `title`; `query` for a matcher |
| `provides.kinds.<kind>.<name>` | `field`: `text`, `url`, `name` or `path` | `extension = "<package>/<name>"` on a kind action | `rows:<kind>` with that field                                 |

A matcher returns `{ index, score?, positions? }` for each candidate it finds, and a sorter every index once, in its order. `dupin.d.ts` declares `Provides`, `Filters`, `FilterInput`, `FilterMatch` and `KindActionData`.

</details>

<details>
<summary>How filters run</summary>

A filter receives the titles of one batch of that source's candidates, up to 1,000 of 512 bytes each, and a matcher also the query. Sorted by a package, rows keep that order and usage history leaves them alone. A filter runs in the package's sandboxed host as you type, with none of the package's capabilities: every host request it makes fails.

Before each batch, Dupin checks with the broker that you allow the package `rows` for the source's kind (`rows:app` for apps, `rows:file` for a folder) with `title`, and `query` for a matcher, and drops the answer if you withdrew either while it ran. A request it isn't yet allowed waits in Settings → Extensions.

If the package isn't running, doesn't provide that filter as that stage, isn't allowed what it needs, takes longer than 250 ms, answers with anything but valid indices, or the source has more than 1,000 candidates, the source runs its own stage for that search, and Dupin logs why once.

</details>

<details>
<summary>How kind actions run</summary>

A kind action is an `actions` export. It appears only on rows Dupin's own sources found: an extension's rows never reach another package's kind action. When you choose it, Dupin asks the broker for `rows:<kind>` with that field, as an action you chose, and runs `actions[name]({ <field>: value })` as any action of the package runs, with its effects. A field it isn't allowed shows why instead.

</details>

## Problems

A mistake in `[advanced]` makes `config.toml` invalid, with the problem named: at launch Dupin uses the defaults, and during a reload it keeps the last valid configuration.

<details>
<summary>What counts as a problem</summary>

A key Dupin doesn't read, a source, matcher or sorter it doesn't have, a stage a source doesn't take, a script name with a `/`, a URL template outside these rules, or a package's filter or action not written `<package>/<name>` makes `config.toml` invalid. So does a `hide` or `order` that names an ID the kind doesn't have, names one twice, or that both lists name, or one on `text` or `url`. During a reload Dupin keeps the last valid configuration, its composition included. A missing script is reported when its action runs.

</details>
