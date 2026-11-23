# Extensions

An extension is a WebAssembly module that answers one keyword. Type its keyword and Space, then a query: Dupin hands the query to the module and shows the rows it returns. Extensions are third-party code, so they are off until you turn them on.

## Turning them on

Switch on **Extensions** in Settings → Search, or set this in `config.toml`:

```toml
extensions = true
```

Dupin then reads `$XDG_CONFIG_HOME/dupin/extensions/` (or `~/.config/dupin/extensions/`). Each folder in it holds one extension. Adding, editing or removing a folder applies while Dupin runs. Settings → Search lists the loaded extensions with their keywords, and every folder it skipped with the reason.

## What an extension can do

A module receives the query and returns up to 50 rows. A row is one of three views: text (title and subtitle), detail (title and body) or pair (source and result). A row may offer up to eight actions; Enter runs the first, → or ⌘K lists them all. An action can only copy text, open an `http` or `https` URL, or open one of the extension's own [views](#views).

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

Typing again cancels the running query. A trap, an exhausted budget or an invalid response becomes a "_Name_ failed" row with the reason; it never affects Dupin or other extensions. [The protocol](plugin-protocol.md) has the full message format and field limits.

## Views

An action whose effect is `push` opens a view of the extension inside the panel. The view is data the extension describes and Dupin draws, as with any other row:

- The chip shows the extension and the view's title, such as **Word Count › Statistics**, and the query field becomes the view's filter, starting empty.
- Dupin asks the same module again, with the view's `view` ID, the filter as `query`, and the `context` the push carried. Typing re-asks it; a newer filter cancels the older query.
- The module answers with a list (`results`, the same rows as at the top) or a `detail`: a title, a long body and up to eight actions.
- Esc, or ⌫ on an empty filter, goes back to what was on screen before the push, with its query and selected row. Esc at the top hides the panel as before.
- A view's actions can push further views, up to eight deep; a push beyond that does nothing.
- A failing query shows a "_Name_ failed" row inside the view. An answer that arrives after its view was left is dropped.

The module keeps nothing between queries, so `context` is how a view knows what it is about: any JSON value up to 4 KiB, nested at most 16 deep. Dupin holds it unchanged and does not read it. A view's filters are not added to query recall, and a view can't be saved as a snapshot yet. Forms are not supported.

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

The word counter offers both: **List words** opens a list of the query's words that typing filters, and **Show statistics** opens a detail page.

## Installing one

```text
~/.config/dupin/extensions/word-count/
├── extension.toml
└── word_count.wasm
```

```toml
id = "word-count" # 1–64 lowercase letters, digits or hyphens, unique
name = "Word Count" # shown on the keyword chip and in Settings
version = "0.1.0"
keyword = "wc" # no spaces
entry = "word_count.wasm" # a .wasm file inside this folder
```

Unknown keys are errors. A keyword that another command already uses (`in`, a Dictionary alias, a web search keyword or another extension) is reported in Settings and the extension is skipped; extensions never take a keyword from anything else.

## Writing one

The module exports a memory named `memory` and three functions:

- `dupin_abi_version() -> i32` returns `1`;
- `dupin_alloc(len: i32) -> i32` returns a buffer of `len` bytes;
- `dupin_query(input: i32, len: i32, output: i32, cap: i32) -> i32` reads the JSON request at `input`, writes the JSON response to `output` and returns its length, or a negative number on failure.

[`examples/extensions/word-count`](../examples/extensions/word-count) is a complete Rust extension. It uses the `dupin-plugin-protocol` crate for the message types. To build and install it with `rustup`:

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

The `command` field carries the extension's `id`. `request_id` and `generation` must be echoed in the response. To test a view, add `"view":"stats","context":{"text":"two words"}` to the request.

## Cost

The runtime ([Wasmi](https://github.com/wasmi-labs/wasmi), an interpreter) adds about 1.2 MiB (6%) to Dupin's stripped release binary. Dupin compiles a module when it loads the folder, so a query pays only for execution. On an M5 Max, the word counter loads in about 1 ms and answers in about 0.1 ms; the calculator fixture from `experiments/plugin-protocol`, an 840 KiB module, loads in about 6.5 ms and answers in 0.2–1.2 ms.
