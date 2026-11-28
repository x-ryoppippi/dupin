# Developing and testing extensions

`dupin ext` scaffolds a package, keeps a development copy live in the launcher, and checks, runs, tests and renders a package without the launcher, in the same sandboxed extension host the launcher uses.

```sh
dupin ext new greeter --ts                 # scaffold a package
dupin ext dev greeter                      # keep a live copy in the extensions folder
dupin ext check greeter                    # load and compile it
dupin ext run greeter --query 'Dupin'      # answer one query
dupin ext test greeter                     # replay its fixtures
dupin ext render greeter greeter/fixtures/search.json search.png
```

| Command                                                                                                      | Does                                                                              |
| ------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------- |
| [`dupin ext new <dir> [--ts \| --bundle]`](#creating-a-package)                                              | Creates a package in a new or empty folder.                                       |
| [`dupin ext dev <dir> [--sourcemap]`](#developing-live)                                                      | Publishes a development copy to the extensions folder and keeps it current.       |
| [`dupin ext types <dir> [--check] [--json]`](#updating-dupindts)                                             | Replaces the package's `dupin.d.ts` with this Dupin's.                            |
| [`dupin ext check <dir>`](#checking-and-running)                                                             | Loads and compiles the package and says what it exports.                          |
| [`dupin ext run <dir> --query Q [--json]`](#checking-and-running)                                            | Answers one query as the launcher would.                                          |
| [`dupin ext test <dir> [--update] [--json]`](#testing-and-rendering)                                         | Replays every fixture and compares each step with its `expect`.                   |
| [`dupin ext render <dir> <fixture> <output.png> [--step N] [--record DIR] [--json]`](#testing-and-rendering) | Renders the panel after a fixture's step, and with `--record` records the replay. |
| [`dupin ext build <dir> [--sourcemap]`](./extension-distribution.md#building-a-bundle)                       | Bundles a package with npm dependencies.                                          |

<details>
<summary>Packages and the host</summary>

Every command compiles and runs the package in the same sandboxed extension host the launcher uses, so a package behaves the same and is just as contained when you or an agent test it. The package's `name` is its id and the name of its folder in `extensions/`; the [manifest](./extension-manifest.md) is the `dupin` key of its `package.json`.

</details>

## Creating a package

`dupin ext new <dir>` creates a package, with a manifest, a `search` that greets the query, `dupin.d.ts` for editors and a fixture. `dupin ext new .` names it after the current folder.

| Flag       | `dupin.main`   | For                                                          |
| ---------- | -------------- | ------------------------------------------------------------ |
| None       | `main.js`      | Plain JavaScript.                                            |
| `--ts`     | `src/main.ts`  | TypeScript that Dupin runs as written.                       |
| `--bundle` | `dist/main.js` | A package with npm dependencies, bundled from `src/main.ts`. |

<details>
<summary>The files it writes</summary>

The folder must be new or empty. Its name becomes the id, so it takes 1–64 lowercase letters, digits or hyphens.

| File                          | Purpose                                                                                                                                                                                 |
| ----------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `package.json`                | The name, version 0.1.0, `"keywords": ["dupin-extension"]` and the `dupin` key, with the id as the keyword and `sf:puzzlepiece.extension` as the [icon](./extension-manifest.md#icons). |
| `main.js` or `src/main.ts`    | A `search` that greets the query: `src/main.ts` with `--ts` or `--bundle`.                                                                                                              |
| `dupin.d.ts`                  | The API's declarations, for editors and `tsc --noEmit`.                                                                                                                                 |
| `fixtures/search.json`        | A [fixture](#fixtures): a query, the row it answers with and what Enter on it does.                                                                                                     |
| `README.md`                   | How to develop, build and share the package.                                                                                                                                            |
| `tsconfig.json`, `.gitignore` | `--ts` and `--bundle`. The compiler sees ES2023 and nothing else, as the guest does.                                                                                                    |
| `rolldown.config.ts`          | `--bundle` only. How `dupin ext build`, `dupin ext dev` and the workflow bundle `src/main.ts`.                                                                                          |
| `.github/workflows/build.yml` | `--bundle` only. Rebuilds `dist/main.js` with Rolldown and commits it on every push to `main`.                                                                                          |

</details>

<details>
<summary>The <code>--bundle</code> scaffold</summary>

With `--bundle`, `package.json` adds `"source": "src/main.ts"`, pins `rolldown` to an exact version in `devDependencies`, and `dupin.main` is `dist/main.js`. Dupin ignores `source`; it tells `dupin ext build` what to bundle. Run `bun install` once and commit the `bun.lock` it writes.

The workflow installs exactly what `bun.lock` pins, with `--frozen-lockfile` and without install scripts, and runs Rolldown on Bun with the same `rolldown.config.ts`. It installs and bundles in a job that can only read the repository and keeps no Git credentials; a second job, which runs no dependency code, holds the write token and commits the bundle.

</details>

<details>
<summary><code>dupin.d.ts</code></summary>

`dupin.d.ts` is a copy of [`sdk/dupin.d.ts`](../../sdk/dupin.d.ts) for the version of Dupin that made it; [`dupin ext types`](#updating-dupindts) brings it up to date. Its `apiVersion` matches the one the host's `dupin` module exports. Everything apiVersion 5 has is in the `dupin` module: `search`, `items(ctx)`, sections, details, accessories and answers, item actions, `views` with their lists, details and forms, `actions` with their effects on pages, `events`, the `filters` and kind actions a package [provides to `[advanced]`](./advanced.md#extensions), the capabilities and `DupinError`.

</details>

## Updating `dupin.d.ts`

`dupin ext types <dir>` replaces the package's `dupin.d.ts` with the one this Dupin ships, so the editor and `tsc` see the API the launcher now offers. `dupin.d.ts` is a generated copy; keep your own declarations in other files. With `--check` it changes nothing and fails with `types-outdated` unless the file already matches, which suits CI.

`dupin ext check` also says when the package's `dupin.d.ts` differs from this Dupin's, unless it declares a newer `apiVersion`, and `--json` reports it as `typesOutdated`. The API grows between `apiVersion` bumps, so a copy of the same version can still lack parts of it.

## Developing live

`dupin ext dev <dir>` publishes a development copy to the extensions folder and keeps it current. A running Dupin with **Extensions folder** on reloads it after each change. Stop `dev` with ⌃C; the last revision stays installed until you delete `extensions/<name>` and `extensions/.dev/<name>`.

<details>
<summary>How it works</summary>

It polls the package, skipping dot-files, `node_modules` and the bundle, and after each change it bundles (if the package has `source`), copies `package.json` and the modules into a new revision, and checks it as `dupin ext check` does. Only a revision that passes goes live: `extensions/<name>` is a symlink to `extensions/.dev/<name>/r<N>`, replaced in one rename, so Dupin never sees a half-written copy. A failed build or check prints its diagnostic and leaves the previous revision live. Each revision is a line on stdout, or with `--json` a JSON object per line.

`dev` refuses to replace an `extensions/<name>` that is not its own symlink, so an installed extension of the same name is safe.

</details>

## Checking and running

```sh
dupin ext check examples/extensions/word-count-js
dupin ext run examples/extensions/word-count-js --query 'two words'
```

`check` compiles the package and says whether it exports `search` or `items`. `run` answers one query as the launcher would; host requests fail as `unavailable`, so use [`dupin ext test`](#testing-and-rendering) to answer them from a fixture. Both exit non-zero on failure.

<details>
<summary>What they do</summary>

`check` loads the folder as the launcher does, has the extension host compile it, runs the module's top-level code and confirms it exports `search` or `items`, and says which. `run` answers one query from `search` and the `items` catalogue, and prints the rows under their sections with their actions. An empty `--query` lists the catalogue. `run` answers no host request: each fails as `unavailable`. To answer them from a fixture and check several steps against stated expectations, use `dupin ext test`.

[`crates/dupin-js-runtime/tests/fixtures`](../../crates/dupin-js-runtime/tests/fixtures) holds extensions that break each limit, for comparison.

</details>

<details>
<summary>JSON output and diagnostics</summary>

```sh
dupin ext run examples/extensions/word-count-js --query 'two words' --json
```

With `--json`, `run` prints the answer in the [protocol](../plugin-protocol.md)'s `results` shape, each row with its `section`, `icon` or `image`, and `detail` beside it and a `run` action's effect as `{"kind": "run", "handler", "data"}`. A failure carries a diagnostic an agent can act on:

```json
{
	"error": "main.js: `items[1].title` must be a string, not a number",
	"diagnostic": {
		"code": "invalid-field",
		"message": "must be a string, not a number",
		"file": "main.js",
		"field": "items[1].title",
		"hint": "Convert values with `String(value)`."
	}
}
```

`code` is stable; `file` is relative to the folder (`package.json` for a manifest problem, with `field` naming the key, such as `dupin.keyword`), and `line` and `column` are present when the problem has a source position.

</details>

## Testing and rendering

```sh
dupin ext test greeter
dupin ext test greeter --update
dupin ext render greeter greeter/fixtures/search.json search.png
```

`dupin ext test` replays every [fixture](#fixtures) in `fixtures/` and compares each step with its `expect`; `--update` rewrites the expectations to what the steps produced. `dupin ext render` renders the panel after step `N` (counting from 1), or the last step, as a PNG. Rendering only draws the panel to a file; it has nothing to do with [Snapshots](./snapshots.md), which save results in the launcher.

```text
FAIL fixtures/search.json
  step 1, query "Dupin":
    rows[0].title: expected "Hello, Dupin!", got "Hi, Dupin!"
1 fixture: 0 passed, 1 failed
```

<details>
<summary>How test runs, and its results</summary>

`test` loads and compiles the package as the launcher does, then runs each step through the sandboxed extension host, with host requests going through the broker as they would in the launcher.

| Flag       | Does                                                                                                                        |
| ---------- | --------------------------------------------------------------------------------------------------------------------------- |
| `--update` | Rewrites each fixture whose steps all ran so every step expects what it produced, and passes it; only when that changes it. |
| `--json`   | Prints the report as JSON.                                                                                                  |

Each difference names the path in the outcome and both values; something the outcome has that the expectation doesn't name is `unexpected`, and something it lacks is `got nothing`. A step that can't run, such as selecting a row the answer doesn't have, stops its fixture. `test` exits non-zero when any fixture fails or can't be read.

`--json` prints `{ "package", "passed", "fixtures": [{ "file", "passed", "updated", "steps": [{ "step", "action", "passed", "outcome", "differences", "problem" }] }] }`, and a fixture that can't be read has a `diagnostic` instead of `steps`.

</details>

<details>
<summary>How render draws</summary>

`render` answers host requests from the fixture's `replies`, through the broker and with the fixture's `consent`, exactly as `test` does, so `search` and `items` show what they would in the launcher.

`render` starts this Dupin headless in a scratch home that holds only a copy of the package's `package.json` and modules, with **Extensions** on and nothing else from your configuration, history or Mac, types each query after the keyword and moves the selection as the steps do, and saves the panel as Dupin draws it. A package Dupin ships for a built-in, such as Calendar or Calculator, answers that built-in as it does in the launcher: the render turns the built-in on, types the query after its keyword, or in the main search for one that answers there, and hands the package the fixture's `preferences` in place of the built-in's settings and only the grants the fixture's `consent` gives. It chooses a step's `push` and `run` actions from the row's action panel, fills in and submits forms and closes pages with the keys a person would press, so the panel shows the page an action opened and the toast it showed; a `clipboard` event reaches the package as a copy does, at its step. Copy, Paste, Open URL and Reveal actions and other events change nothing on screen and aren't replayed. To show what the package had stored before the first step, seed `replies.storage` with it.

The fixture's theme, panel width, scale and, if it gives one, height are used; otherwise the panel's own height. The guest sees the fixture's clock, seed and time zone, and Dupin says how long ago a row's `time` was by the same clock, so two renders of one fixture on one Mac are the same bytes on any day. Every build of Dupin can render, the released app included; only a build made with `--no-default-features` leaves Rune's headless renderer out, and `render` then fails with `render-failed`. `--json` prints `{ "png", "step", "width", "height", "scale", "shown" }`, where `shown` is every text the panel shows, in the order it lays them out.

`--record DIR` also records the replay, up to the same step, as `DIR/frame_0000.png` and on, 10 frames a second at the fixture's scale and at least 720 points tall, so the panel has room to grow, with the keys it pressed in `DIR/script.json` (Rune's script format). A recording presses keys at a pace a reader can follow, slower than a still needs; the still at `output.png` is the same either way. `scripts/docs-images.nu` turns such a recording into a clip for these docs.

</details>

## Fixtures

A fixture is one JSON file in `fixtures/`, read in name order: a format version, what the host services answer, and the steps with what each should produce.

```json
{
	"fixture": 1,
	"steps": [{ "query": "Dupin", "expect": { "rows": [{ "id": "#0", "title": "Hello, Dupin!" }] } }]
}
```

| Key           | Value                                                                                |
| ------------- | ------------------------------------------------------------------------------------ |
| `fixture`     | Required. The format version, `1`.                                                   |
| `description` | Optional text for people.                                                            |
| `environment` | Optional. The clock, time zone and render settings; see [Environment](#environment). |
| `replies`     | Optional. What the host services answer; see [Replies](#replies).                    |
| `preferences` | Optional. What every call sees in `ctx.preferences`.                                 |
| `steps`       | Required. 1–64 [steps](#steps), the first a `query`.                                 |

<details>
<summary>Every key, and a fuller example</summary>

Symbolic links are refused, so a fixture can't make the runner read outside the package.

```json
{
	"fixture": 1,
	"description": "Shows whether the repository is archived.",
	"environment": { "now": "2026-01-01T09:00:00Z", "timeZone": "Europe/London" },
	"replies": {
		"http": [
			{
				"request": { "url": "https://api.github.com/repos/x-ryoppippi/dupin" },
				"response": { "json": { "archived": false } }
			}
		]
	},
	"steps": [
		{
			"query": "dupin",
			"expect": { "rows": [{ "id": "#0", "title": "Active repository", "actions": [] }] }
		},
		{ "select": 0 }
	]
}
```

| Key           | Value                                                                                                                                                                                   |
| ------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `fixture`     | Required. The format version, `1`. A fixture of another version is reported, not guessed at.                                                                                            |
| `description` | Optional text for people.                                                                                                                                                               |
| `environment` | Optional. What the run would otherwise take from this Mac; see [Environment](#environment).                                                                                             |
| `replies`     | Optional. What the host services answer; see [Replies](#replies). Left out, every host request fails as `unavailable`.                                                                  |
| `preferences` | Optional. What every call sees in [`ctx.preferences`](./extension-manifest.md#preferences) over the declared defaults, such as a bundled port's config.toml settings. `{}` if left out. |
| `steps`       | Required. 1–64 steps, the first a `query`. Each has exactly one of `query`, `select`, `activate`, `event`, `back`, `fill` and `submit`, and `expect`.                                   |

A fixture's `preferences.snippets`, each with a `source` of `config` or `stored`, are what the [`snippets`](./extension-permissions.md#snippets) service checks and changes; a change is handed back as the next calls' preferences, as Dupin hands it to the shipped package.

</details>

### Steps

Each step does one thing and may `expect` what it produced; only the keys `expect` names are compared.

| Step                                  | Does                                              |
| ------------------------------------- | ------------------------------------------------- |
| `{ "query": "…" }`                    | Types a query and waits for the answer.           |
| `{ "select": 2 }`                     | Selects a row, counting from 0.                   |
| `{ "activate": "Copy" }`              | Chooses the selected row's action by title or id. |
| `{ "event": "clipboard", "data": … }` | Delivers an event to the package.                 |
| `{ "back": true }`                    | Closes the page on top, as Esc does.              |
| `{ "fill": { "title": "Tea" } }`      | Fills in the form on screen by field ID.          |
| `{ "submit": true }`                  | Submits the form on screen, as ⌘Enter does.       |

<details>
<summary>What each step does and produces</summary>

| Step                                  | Does                                                                                                                                                                                                         | Its outcome                                                                                                                              |
| ------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `{ "query": "…" }`                    | Types the query after the keyword, or on the page on top, replacing the last, and waits for the answer. The first row is selected.                                                                           | `rows`, `selected` when there are rows, `view` on a page, and `error` when the search or view failed.                                    |
| `{ "select": 2 }`                     | Selects a row, counting from 0.                                                                                                                                                                              | `selected`                                                                                                                               |
| `{ "activate": "Copy" }`              | Chooses the selected row's action with this title or id. A `run` action calls the package's `actions` handler as Enter does, and a `push` opens its view; Copy and Open URL are recorded, never carried out. | `effect`, and for `run`, `toasts` or `error`; for a `push`, or a `run` that moved or changed a page, also `rows`, `selected` and `view`. |
| `{ "event": "clipboard", "data": … }` | Calls `events[event](data)`, as a watch Dupin keeps for the package would.                                                                                                                                   | `invalidated` when the handler invalidated the catalogue, `error` when it failed.                                                        |
| `{ "back": true }`                    | Closes the page on top, as Esc does.                                                                                                                                                                         | `rows`, `selected` and, still on a page, `view`.                                                                                         |
| `{ "fill": { "title": "Tea" } }`      | Fills in the form on screen by field ID: a field's text, a checkbox's `true` or `false`, or the `value` of a select's option.                                                                                | `rows` and `view`.                                                                                                                       |
| `{ "submit": true }`                  | Submits the form on screen, as ⌘Enter does: Dupin's own checks first, then the form's `submit` action.                                                                                                       | `view` with each field's `error`, or what the action did, as for `activate`.                                                             |

Any step's outcome also has `clipboard`, the texts the package wrote to the clipboard during it, `windows`, each frame it moved the focused window to, and `storage`, the whole store as each record's `data`, oldest first and without blobs, when the step changed it.

`expect` names any of `rows`, `selected`, `effect`, `toasts`, `panel` (`"hide"` or `"paste"` when an action hides the panel, `{ "enter": { "command", "query" } }` when it enters another command), `clipboard`, `windows`, `storage`, `invalidated`, `error` and `view`. Only the keys it names are compared, each as canonical JSON, and a key the outcome lacks compares as `null`.

</details>

<details>
<summary>Outcome shapes</summary>

- `view` is the page on top: `{ "name", "title", "depth", "state", "type", "emptyText" }`, where `type` is `list`, `detail` or `form` and `state` and `emptyText` show only when the page has them. A form's also has `submit`, its submit key's title, and `fields`, each `{ "id", "type", "title", "value", "required", "error" }`, with `required` and `error` only when they apply. A detail page's one row, `detail`, carries its title, detail and actions.
- A row is `{ "id", "title", "subtitle", "answer", "accessories", "aliases", "keywords", "icon", "image", "section", "detail", "actions" }`, each optional field only when the row has it, where `detail` is `{ "image", "markdown", "metadata" }` and each action is `{ "title", "copy" | "paste" | "url" | "run" | "push" }`, with `"template": true` for a template, whose text the outcome shows unfilled. `id` is the item's own or `#0`, `#1` and so on.
- An `error` is the diagnostic: `code`, `message`, `file`, `field`, `line`, `column` and `hint`, as `dupin ext run --json` reports it.

</details>

<details>
<summary>How steps run</summary>

A step's `activate` may name an action by the ID the launcher gives it: `copy`, `paste` and `open` for a row's first Copy, Paste and Open URL, and `action-<index>` for the others.

A query step answers as the keyword does: `search`'s rows for a query that isn't blank, then the `items()` catalogue's matches, ranked by Dupin's own matcher (without usage history, which a fixture doesn't have) and losing their `section` once the query ranks them, then its web searches filled with the query.

The fixture holds the catalogue from step to step as the launcher does, so an `activate` or `event` step whose handler calls `items.invalidate()` has `invalidated: true` in its outcome, and the next query asks `items` with `reason` `"invalidated"` and the cursor and applies the update it answers. A package in the folder Dupin's bundled extensions are in, such as the repository's `extensions/`, replays as that bundled package, as the launcher runs it: with its own storage, and entering another package's command, which only a bundled package may.

</details>

### Environment

| `environment` key | Default                        | Fixes                            |
| ----------------- | ------------------------------ | -------------------------------- |
| `now`             | `"2026-01-01T09:00:00Z"`       | The clock, which doesn't move.   |
| `seed`            | `1`                            | `Math.random`.                   |
| `timeZone`        | `"UTC"`                        | The extension host's time zone.  |
| `locale`          | `"en-GB"`                      | `LANG` for the headless render.  |
| `theme`           | `"dark"`                       | `render`'s appearance.           |
| `window`          | `{ "width": 680, "scale": 2 }` | `render`'s panel size and scale. |

<details>
<summary>Every key in full</summary>

| `environment` key | Default                        | Fixes                                                                                                      |
| ----------------- | ------------------------------ | ---------------------------------------------------------------------------------------------------------- |
| `now`             | `"2026-01-01T09:00:00Z"`       | The clock, in UTC with optional milliseconds. `Date.now()` and `new Date()` return it and it doesn't move. |
| `seed`            | `1`                            | `Math.random`, which returns the same numbers for the same seed.                                           |
| `timeZone`        | `"UTC"`                        | The extension host's time zone, a tz database name this Mac has, for local-time `Date` methods.            |
| `locale`          | `"en-GB"`                      | `LANG` for the headless render. The guest has no `Intl`, so no answer depends on it yet.                   |
| `theme`           | `"dark"`                       | `render`'s appearance, `dark` or `light`.                                                                  |
| `window`          | `{ "width": 680, "scale": 2 }` | `render`'s panel width (400–1200 points), scale (1–3) and, optionally, `height` (60–2000 points).          |

</details>

### Replies

`replies` answers host requests in place of this Mac, by service: `http`, `files`, `exec`, `storage`, `clipboard`, `dictionary`, `calculator`, `calendar`, `windows` and `context`, plus `consent` for your decision on each scope. A request with no reply fails rather than reaching the network, a program or a file of yours.

<details>
<summary>Every reply</summary>

Every request still goes through the broker, against the scopes `package.json` declares, so an undeclared origin, a method or path out of scope, or a clipboard read while typing fails exactly as in the launcher, before any reply is looked up.

| `replies` key | Value                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| ------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `http`        | `[{ "request": { "method", "url" }, "response": { "status", "headers", "body" or "json" } }]`. `method` defaults to `GET` and `status` to `200`; the URL must match exactly. Any other request fails with `network`.                                                                                                                                                                                                                                                                                              |
| `files`       | Each declared `files` scope's contents by name: a string for a `file` scope, or an object of relative paths and texts for a `directory` scope. Each scope is bound to a scratch folder holding exactly these files, empty when left out. `files.root` reports a scope that names its `path` under `~/` as if the home folder were `/Users/dupin`, and any other as `/fixture/<name>`, so its answers are the same on every Mac; an undecided scope that names its path is missing, as a folder this Mac lacks is. |
| `exec`        | `[{ "profile", "args", "status", "stdout", "stderr" }]`. `profile` is the declared name, `@script:` included; `args`, if given, must equal the arguments after the inputs fill the template. Nothing is run.                                                                                                                                                                                                                                                                                                      |
| `storage`     | The `data` of each record the package's storage holds when the fixture starts, oldest first, stored within its declared quota in a scratch store that ends with the fixture.                                                                                                                                                                                                                                                                                                                                      |
| `clipboard`   | What a clipboard read returns, such as `{ "type": "text", "text": "…" }`, or `null` for an empty clipboard. Watched copies come as `event` steps.                                                                                                                                                                                                                                                                                                                                                                 |
| `dictionary`  | Each term's entries, in the [dictionary](./extension-permissions.md#dictionary)'s shape. Any other term fails with `unavailable`.                                                                                                                                                                                                                                                                                                                                                                                 |
| `calculator`  | Each expression's answer, in the [calculator](./extension-permissions.md#calculator)'s shape, or `null` for one it doesn't answer. Any other expression fails with `unavailable`.                                                                                                                                                                                                                                                                                                                                 |
| `calendar`    | The events a read can see, in the [calendar](./extension-permissions.md#calendar)'s shape; a read returns those overlapping its range that match its `query`, in its `order`. Whole days are counted from `now` in UTC.                                                                                                                                                                                                                                                                                           |
| `windows`     | `{ "displays": [...], "focused": … }`: the displays and the focused window, in the [windows](./extension-permissions.md#windows) service's shape, or `null` for no window. A move goes through the same bounds as on a Mac and updates `focused`, so the next step sees where it went.                                                                                                                                                                                                                            |
| `context`     | `{ "frontmost": …, "document": … }`: the app and the file in the [context](./extension-permissions.md#context) service's shapes, each `null` or left out for none.                                                                                                                                                                                                                                                                                                                                                |
| `consent`     | Your decision on each declared scope, named as grant files name it: `"allow"`, `"deny"` (granted, then revoked) or `"pending"` (`needs-consent`). A scope left out is allowed, and macOS access counts as allowed. Naming a scope the package doesn't declare is an error.                                                                                                                                                                                                                                        |

</details>

## Through MCP

[`dupin mcp --workspace DIR`](./mcp.md#extension-tools) offers `ext_check`, `ext_run`, `ext_test` and `ext_render` to an agent, for packages inside `DIR`.

<details>
<summary>How they differ from the commands</summary>

They are the commands above, offline: no build runs, nothing is written to the workspace (`ext_test` never updates), no grant you made applies and nothing asks macOS for access. `ext_render` returns the PNG itself.

</details>
