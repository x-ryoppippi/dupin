# Extensions

An extension is a folder that answers one keyword. Type its keyword and Space, then a query: Dupin hands the query to the extension and shows the rows it returns. An extension is a JavaScript module, or a WebAssembly module for the older guest interface. Extensions are third-party code, so they are off until you turn them on.

## Turning them on

`extensions` turns on extensions: third-party folders in `extensions/` beside `config.toml`, each answering its own keyword. It is off by default; Settings → Search has the same switch.

```toml
extensions = false # run extensions from the extensions folder beside config.toml
```

Switch on **Extensions** in Settings → Search, or set this in `config.toml`:

```toml
extensions = true
```

Dupin then reads `$XDG_CONFIG_HOME/dupin/extensions/` (or `~/.config/dupin/extensions/`). Each folder in it holds one extension. Adding, editing or removing a folder, or any JavaScript file in it, applies while Dupin runs. Settings → Search lists the loaded extensions with their keywords, and every folder it skipped with the reason.

## A JavaScript extension

Two files make a complete extension:

```text
~/.config/dupin/extensions/word-count/
├── package.json
└── main.js
```

```json
{
	"name": "word-count",
	"version": "0.1.0",
	"dupin": { "title": "Word Count", "keyword": "wc" }
}
```

```js
export function search(query) {
	const input = query.trim();
	if (!input) return [];

	const count = input.split(/\s+/u).length;
	const title = `${count} words`;
	const text = String(count);

	return [{ title, text }];
}
```

Type `wc two words` to see **2 words**. Enter copies `2`. [`examples/extensions/word-count-js`](../../examples/extensions/word-count-js) holds these two files.

The manifest is the `dupin` key of `package.json`, so a package keeps one file for Dupin and its own JavaScript tooling. Dupin reads it as plain JSON before running any code:

| Key                  | Value                                                                                                                                 |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| `name`               | Required. The extension's id: 1–64 lowercase letters, digits or hyphens, unique among extensions. No npm scopes.                      |
| `version`            | Required. 1–32 characters without spaces.                                                                                             |
| `dupin.title`        | Required. Shown on the keyword chip and in Settings, up to 80 characters.                                                             |
| `dupin.keyword`      | Required. Up to 32 bytes, no spaces.                                                                                                  |
| `dupin.main`         | Optional. The module that exports `search`, a `.js` or `.mjs` file inside the folder, such as `dist/index.js`. Defaults to `main.js`. |
| `dupin.capabilities` | Reserved. Dupin grants no capabilities yet, so any entry in it is an error.                                                           |

Any other key inside `dupin` is an error. Every other key of `package.json`, such as `scripts`, `dependencies` or `devDependencies`, is ignored. A folder whose `package.json` has no `dupin` key is not an extension, and Settings says so. A keyword that another command already uses (`in`, a Dictionary alias, a web search keyword or another extension) is reported in Settings and the extension is skipped; extensions never take a keyword from anything else.

### `search(query, ctx)`

The module exports `search`, a function or an `async` function. It receives the text typed after the keyword and a frozen `ctx` whose `limit` is the number of rows Dupin will show. It returns an array of items, or a list, `{ type: "list", items: [...] }` (`type` may be left out). Return `[]` for no results.

| Item field | Value                                                                                     |
| ---------- | ----------------------------------------------------------------------------------------- |
| `title`    | Required. One line, up to 512 bytes.                                                      |
| `subtitle` | Optional, up to 2,048 bytes.                                                              |
| `id`       | Optional and unique among the items, up to 128 bytes. Dupin numbers items without one.    |
| `url`      | Optional. An `http` or `https` URL with a host, up to 2,048 bytes: **Open URL** opens it. |
| `text`     | Optional, up to 16 KiB: **Copy** copies it.                                               |

Enter runs the first action: **Open URL** when the item has a `url`, otherwise **Copy**. → or ⌘K lists both. An item with neither shows but does nothing. Every value is checked; an unknown field, a value of the wrong type, an empty `text` or a URL with another scheme fails the whole answer with a row that says which item and field.

### What the module can reach

The module is plain modern JavaScript ([QuickJS-ng](https://quickjs-ng.github.io/quickjs/es_features/)): modules, `async`/`await`, regular expressions, `Date` and `JSON`, but no `Intl`. It can import other `.js` or `.mjs` files of its own folder with relative paths that include the extension (`import { rows } from "./lib/rows.js"`), and the host module `dupin`, which for now exports only `apiVersion`. Nothing else resolves: no packages by name, no URLs, no absolute paths and no path or symlink that leads outside the folder.

There are no files, network, processes, clipboard, timers or console. Each query runs in a fresh engine, on the extension's own thread, so module-level variables never carry from one query to the next.

TypeScript, JSX and npm packages are built on your machine: bundle the extension into plain JavaScript (for example with `esbuild main.ts --bundle --format=esm --outfile=dist/main.js`), point `dupin.main` at the output and install the folder. Dupin runs JavaScript only, never type-checks and never runs a package manager or the package's scripts.

| Limit                        | Value          |
| ---------------------------- | -------------- |
| JavaScript files             | 256 and 4 MiB  |
| Heap                         | 16 MiB         |
| Stack                        | 512 KiB        |
| Time per query               | 2 seconds      |
| Promise jobs per query       | 10,000         |
| Items and text in one answer | 50 and 256 KiB |
| Extensions read              | 64 folders     |

Typing again cancels the running query. A thrown error, a rejected or never-settling promise, an exhausted limit or an invalid answer becomes a "_Name_ failed" row with the file, line or field at fault; it never affects Dupin or other extensions. Dupin compiles every JavaScript file when it loads the folder, so a syntax error or a broken import is reported in Settings before the first query.

### Testing one

Check and run a folder without the launcher:

```sh
dupin ext check examples/extensions/word-count-js
dupin ext run examples/extensions/word-count-js --query 'two words'
dupin ext run examples/extensions/word-count-js --query 'two words' --json
```

`check` loads the folder as the launcher does, runs the module's top-level code and confirms it exports `search`. `run` asks it one query and prints the rows and their actions, or with `--json` the answer in the [protocol](../plugin-protocol.md)'s `results` shape. Both exit non-zero on failure. With `--json`, a failure carries a diagnostic an agent can act on:

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

`code` is stable; `file` is relative to the folder (`package.json` for a manifest problem, with `field` naming the key, such as `dupin.keyword`), and `line` and `column` are present when the problem has a source position. [`crates/dupin-js-runtime/tests/fixtures`](../../crates/dupin-js-runtime/tests/fixtures) holds extensions that break each limit, for comparison.

## Authoring

`dupin ext` has three commands for writing an extension. They describe a package with `package.json`, whose `dupin` key is the manifest: `title`, `keyword` and optionally `main`, the module Dupin runs, which defaults to `main.js`. The package's `name` is its id and the name of its folder in `extensions/`.

### `dupin ext new <dir> [--ts]`

Creates a package in a new or empty folder. The folder's name becomes the id, so it takes 1–64 lowercase letters, digits or hyphens.

| File                          | Purpose                                                                                                     |
| ----------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `package.json`                | The name, version 0.1.0, `"keywords": ["dupin-extension"]` and the `dupin` key, with the id as the keyword. |
| `main.js` or `src/main.ts`    | A `search` that greets the query.                                                                           |
| `dupin.d.ts`                  | The API's declarations, for editors and `tsc --noEmit`.                                                     |
| `fixtures/search.json`        | A query and the items `search` answers it with.                                                             |
| `README.md`                   | How to develop, build and share the package.                                                                |
| `tsconfig.json`, `.gitignore` | `--ts` only. The compiler sees ES2023 and nothing else, as the guest does.                                  |
| `.github/workflows/build.yml` | `--ts` only. Rebuilds `dist/main.js` with Bun and commits it on every push to `main`.                       |

With `--ts`, `package.json` adds `"source": "src/main.ts"` and `dupin.main` is `dist/main.js`. Dupin ignores `source`; it tells `dupin ext build` what to bundle.

`dupin.d.ts` is a copy of [`sdk/dupin.d.ts`](../../sdk/dupin.d.ts) for the version of Dupin that made it. Its `apiVersion` matches the one the host's `dupin` module exports. Everything apiVersion 1 has is in the `dupin` module; what the [design](../design/extension-platform.md) plans next, such as `items(ctx)`, sections, Detail and Form views and the `http`, `files`, `exec`, `storage`, `clipboard`, `dictionary` and `calendar` capabilities, is drafted as types in its `v2` namespace. Nothing in `v2` works yet: an item with those fields fails the answer, and a manifest that declares capabilities is an error.

### `dupin ext build <dir> [--sourcemap]`

A package with `source` is bundled into a single ES module at `dupin.main`, with its npm dependencies, by `bun` or, without it, `esbuild` from `PATH`. Dupin never downloads either; with neither installed the build fails and says so. `--sourcemap` also writes `<main>.map` and links it from the bundle. The bundle is then compiled and linked, and its top-level code run, as the launcher loads a package.

A dependency that imports a Node built-in (`fs`, `net`, `child_process` and the rest, with or without `node:`) fails the build, naming the file that imports it. Bundlers otherwise quietly stub or polyfill some built-ins for the browser, and the guest has none of them. A failed build or check puts back the `dupin.main` that was there before.

A package without `source` is plain JavaScript and needs no build; `build` only checks it. Dupin installs packages from Git without building them, so commit the bundle; the `--ts` scaffold's workflow does that for you.

### `dupin ext dev <dir> [--sourcemap]`

Publishes a development copy to the extensions folder and keeps it current. It polls the package, skipping dot-files, `node_modules` and the bundle, and after each change it bundles (if the package has `source`), copies `package.json` and the JavaScript into a new revision, and checks it. Only a revision that passes goes live: `extensions/<name>` is a symlink to `extensions/.dev/<name>/r<N>`, replaced in one rename, so a running Dupin with **Extensions** on reloads it and never sees a half-written copy. A failed build or check prints its diagnostic and leaves the previous revision live. Each revision is a line on stdout, or with `--json` a JSON object per line.

`dev` refuses to replace an `extensions/<name>` that is not its own symlink, so an installed extension of the same name is safe. Stop `dev` with ⌃C; the last revision stays installed until you delete `extensions/<name>` and `extensions/.dev/<name>`.

## A Wasm extension

A folder with `extension.toml` instead of `package.json` runs a WebAssembly module. A module receives the query and returns up to 50 rows. A row is one of three views: text (title and subtitle), detail (title and body) or pair (source and result). Bodies are plain text; list sections and Markdown details are host features that built-ins such as Calendar and Dictionary use, and extensions cannot return them yet. A row may offer up to eight actions; Enter runs the first, → or ⌘K lists them all. An action can only copy text, open an `http` or `https` URL, or open one of the extension's own [views](#views).

Nothing else is reachable. A module that imports anything, including WASI, is rejected, so it cannot read files, use the network, start processes, read the clipboard or see the time. Each query runs in a fresh instance, on its own thread, with no memory of earlier queries.

| Limit                    | Value                 |
| ------------------------ | --------------------- |
| Module size              | 4 MiB                 |
| Linear memory            | 16 MiB                |
| Execution per query      | 20 million fuel units |
| Time per query           | 2 seconds             |
| Request and response     | 16 KiB and 256 KiB    |
| Rows and actions per row | 50 and 8              |
| Extensions read          | 64 folders            |

Typing again cancels the running query. A trap, an exhausted budget or an invalid response becomes a "_Name_ failed" row with the reason; it never affects Dupin or other extensions. [The protocol](../plugin-protocol.md) has the full message format and field limits.

### Views

An action whose effect is `push` opens a view of the extension inside the panel. The view is data the extension describes and Dupin draws, as with any other row:

- The chip shows the extension and the view's title, such as **Word Count › Statistics**, and the query field becomes the view's filter, starting empty.
- Dupin asks the same module again, with the view's `view` ID, the filter as `query`, and the `context` the push carried. Typing re-asks it; a newer filter cancels the older query.
- The module answers with a list (`results`, the same rows as at the top) or a `detail`: a title, a long body and up to eight actions.
- Esc, or ⌫ on an empty filter, goes back to what was on screen before the push, with its query and selected row. Esc at the top hides the panel as before.
- A view's actions can push further views, up to eight deep; a push beyond that does nothing.
- A failing query shows a "_Name_ failed" row inside the view. An answer that arrives after its view was left is dropped.

The module keeps nothing between queries, so `context` is how a view knows what it is about: any JSON value up to 4 KiB, nested at most 16 deep. Dupin holds it unchanged and does not read it. A view's filters are not added to query recall, and a view can't be saved as a snapshot yet. Forms are not supported. JavaScript extensions can't open views yet.

```json
{
	"id": "stats",
	"title": "Show statistics",
	"effect": {
		"kind": "push",
		"view": "stats",
		"title": "Statistics",
		"context": { "text": "two words" }
	}
}
```

```json
{
	"kind": "detail",
	"title": "2 words",
	"body": "Words: 2\nCharacters: 9",
	"actions": [
		{ "id": "copy", "title": "Copy statistics", "effect": { "kind": "copy", "text": "Words: 2" } }
	]
}
```

The Wasm word counter offers both: **List words** opens a list of the query's words that typing filters, and **Show statistics** opens a detail page.

### Installing and writing one

```text
~/.config/dupin/extensions/word-count/
├── extension.toml
└── word_count.wasm
```

```toml
id = "word-count"
name = "Word Count"
version = "0.1.0"
keyword = "wc"
entry = "word_count.wasm" # a .wasm file inside this folder
```

`extension.toml` describes Wasm modules only: its `entry` is required and must name a `.wasm` file, and unknown keys are errors. A folder with both `extension.toml` and a `package.json` with a `dupin` key is skipped as ambiguous; any other `package.json`, including one Dupin can't parse, is left to your tooling and does not get in the way.

The module exports a memory named `memory` and three functions:

- `dupin_abi_version() -> i32` returns `1`;
- `dupin_alloc(len: i32) -> i32` returns a buffer of `len` bytes;
- `dupin_query(input: i32, len: i32, output: i32, cap: i32) -> i32` reads the JSON request at `input`, writes the JSON response to `output` and returns its length, or a negative number on failure.

[`examples/extensions/word-count`](../../examples/extensions/word-count) is a complete Rust extension. It uses the `dupin-plugin-protocol` crate for the message types. To build and install it with `rustup`:

```sh
rustup target add wasm32-unknown-unknown
cd examples/extensions/word-count
cargo build --release --target wasm32-unknown-unknown
mkdir -p ~/.config/dupin/extensions/word-count
cp extension.toml ~/.config/dupin/extensions/word-count/
cp target/wasm32-unknown-unknown/release/dupin_word_count.wasm \
  ~/.config/dupin/extensions/word-count/word_count.wasm
```

Inside this repository's Nix dev shell, the toolchain has the target but no Wasm linker; prefix the build with `nix shell --inputs-from . nixpkgs#lld --command`.

Test a module without the launcher:

```sh
printf '%s' '{"version":1,"request_id":"1","generation":"1","command":"word-count","query":"two words","limit":5,"state":null}' > request.json
dupin ext check word_count.wasm request.json --json
dupin ext snap word_count.wasm request.json view.png
```

The `command` field carries the extension's `id`. `request_id` and `generation` must be echoed in the response. To test a view, add `"view":"stats","context":{"text":"two words"}` to the request. `dupin ext check` and `dupin ext run` also take the installed folder, as for a JavaScript extension.

## Cost

The JavaScript engine ([QuickJS-ng](https://quickjs-ng.github.io/quickjs/) through [`rquickjs`](https://github.com/DelSkayn/rquickjs)) is compiled into Dupin; standalone size-optimized builds put it at about 0.6 MiB. Dupin compiles a package to bytecode when it loads the folder, so a query starts a fresh engine from bytecode and pays only for execution.

The Wasm runtime ([Wasmi](https://github.com/wasmi-labs/wasmi), an interpreter) adds about 1.2 MiB (6%) to Dupin's stripped release binary. Dupin compiles a module when it loads the folder, so a query pays only for execution. On an M5 Max, the word counter loads in about 1 ms and answers in about 0.1 ms; the calculator fixture from `experiments/plugin-protocol`, an 840 KiB module, loads in about 6.5 ms and answers in 0.2–1.2 ms.
