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
├── extension.toml
└── main.js
```

```toml
id = "word-count" # 1–64 lowercase letters, digits or hyphens, unique
name = "Word Count" # shown on the keyword chip and in Settings
version = "0.1.0"
keyword = "wc" # no spaces
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

The manifest may also name the module with `entry = "src/index.js"`, a `.js` or `.mjs` file inside the folder; without it, Dupin runs `main.js`. Unknown keys are errors. A keyword that another command already uses (`in`, a Dictionary alias, a web search keyword or another extension) is reported in Settings and the extension is skipped; extensions never take a keyword from anything else.

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

There are no files, network, processes, clipboard, timers or console. Each query runs in a fresh engine, so module-level variables never carry from one query to the next. The engine runs in a separate, sandboxed process; see [Security model](#security-model).

TypeScript, JSX and npm packages are built on your machine: bundle the extension into plain JavaScript (for example with `esbuild main.ts --bundle --format=esm --outfile=main.js`) and install the output. Dupin runs JavaScript only and never type-checks.

| Limit                        | Value                             |
| ---------------------------- | --------------------------------- |
| JavaScript files             | 256 and 4 MiB                     |
| Heap                         | 16 MiB                            |
| Stack                        | 512 KiB                           |
| Time per query               | 2 seconds                         |
| Promise jobs per query       | 10,000                            |
| Items and text in one answer | 50 and 256 KiB                    |
| Extensions read              | 64 folders                        |
| Extension host memory        | 64 MiB plus the bytecode it holds |

Typing again cancels the running query. A thrown error, a rejected or never-settling promise, an exhausted limit or an invalid answer becomes a "_Name_ failed" row with the file, line or field at fault; it never affects Dupin or other extensions. Dupin compiles every JavaScript file when it loads the folder, so a syntax error or a broken import is reported in Settings before the first query.

### Security model

JavaScript extension code never runs inside Dupin, though Dupin parses and compiles it there when it loads the folder. The first query starts the extension host, Dupin's own executable run with `--extension-host`, and every JavaScript extension shares it. Before it runs any extension code, the host confines itself with a deny-by-default macOS sandbox: it cannot open or list files, use the network, connect to other processes or system services, or start programs. Its resource limits allow no new files, child processes or core dumps. Dupin sends it each package's compiled bytecode and each query over a private socket, so the host never reads the extensions folder, and it checks every answer again before showing it. The host keeps the time zone it starts with, so `Date` reports local time.

The sandbox contains an extension that breaks out of the engine through a flaw in QuickJS-ng, but because every JavaScript extension shares the host, such an extension could see the other extensions' code and queries and change their answers until the host is replaced. Install only extensions you trust. A host that sends anything Dupin didn't ask for is stopped.

The engine stops a query that runs past its deadline or is cancelled, but a few built-in operations, such as `indexOf` on very long strings, can't be interrupted, and the engine's heap limit doesn't bound the whole process. Dupin therefore watches the host from outside. It stops the host when a query runs 150 ms past its deadline, ignores a cancellation for 150 ms or grows past its memory allowance, and when the host dies, the query becomes a "_Name_ failed" row. The next query starts a new host. One query runs at a time; the others wait in a bounded queue, and a query that typing replaces leaves it. The host exits when Dupin quits and after two minutes without queries.

The sandbox exists on macOS only. On other platforms the host refuses to run extensions and each query fails with `sandbox-unavailable`; Linux needs its own confinement first.

### Testing one

Check and run a folder without the launcher:

```sh
dupin ext check examples/extensions/word-count-js
dupin ext run examples/extensions/word-count-js --query 'two words'
dupin ext run examples/extensions/word-count-js --query 'two words' --json
```

`check` loads the folder as the launcher does, runs the module's top-level code and confirms it exports `search`. Both run the module in the sandboxed extension host, as the launcher does, so a package behaves the same and is just as contained when you or an agent test it. `run` asks it one query and prints the rows and their actions, or with `--json` the answer in the [protocol](../plugin-protocol.md)'s `results` shape. Both exit non-zero on failure. With `--json`, a failure carries a diagnostic an agent can act on:

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

`code` is stable; `file` is relative to the folder, and `line` and `column` are present when the problem has a source position. [`crates/dupin-js-runtime/tests/fixtures`](../../crates/dupin-js-runtime/tests/fixtures) holds extensions that break each limit, for comparison.

## A Wasm extension

A folder whose `entry` names a `.wasm` file runs a WebAssembly module instead. A module receives the query and returns up to 50 rows. A row is one of three views: text (title and subtitle), detail (title and body) or pair (source and result). Bodies are plain text; list sections and Markdown details are host features that built-ins such as Calendar and Dictionary use, and extensions cannot return them yet. A row may offer up to eight actions; Enter runs the first, → or ⌘K lists them all. An action can only copy text, open an `http` or `https` URL, or open one of the extension's own [views](#views).

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

The JavaScript engine ([QuickJS-ng](https://quickjs-ng.github.io/quickjs/) through [`rquickjs`](https://github.com/DelSkayn/rquickjs)) is compiled into Dupin at `-O3`, which adds about 1 MiB to the signed app; the extension host is the same executable, so it adds no file. Dupin compiles a package to bytecode when it loads the folder, so a query starts a fresh engine from bytecode and pays only for execution. On an M5 Max shared with other build jobs, a 400-candidate search returning 50 rows took 1.4 ms at the median and 1.7 ms at p95 over 2,000 queries to a running host. The first query, which starts the host, took 17 ms at the median and 30 ms at p95, most of it the cost of starting Dupin's executable. The idle host's footprint was about 6.5 MiB. `cargo test --release -p dupin --test extension_host -- --ignored --nocapture` repeats the measurement.

The Wasm runtime ([Wasmi](https://github.com/wasmi-labs/wasmi), an interpreter) adds about 1.2 MiB (6%) to Dupin's stripped release binary. Dupin compiles a module when it loads the folder, so a query pays only for execution. On an M5 Max, the word counter loads in about 1 ms and answers in about 0.1 ms; the calculator fixture from `experiments/plugin-protocol`, an 840 KiB module, loads in about 6.5 ms and answers in 0.2–1.2 ms.
