# Extensions

An extension is a folder that answers one keyword. Type its keyword and Space, then a query: Dupin hands the query to the extension and shows the rows it returns. An extension is a JavaScript module. Some extensions ship with Dupin and are on from the start; any others are third-party code, so they are off until you turn them on.

## Turning them on

`extensions` turns on the extensions folder: third-party folders in `extensions/` beside `config.toml`, each answering its own keyword. It is off by default; **Extensions folder** in Settings → Extensions is the same switch.

```toml
extensions = false # run extensions from the extensions folder beside config.toml
```

Switch on **Extensions folder** in Settings → Extensions, or set this in `config.toml`:

```toml
extensions = true
```

Dupin then reads `$XDG_CONFIG_HOME/dupin/extensions/` (or `~/.config/dupin/extensions/`). Each folder in it holds one extension. Adding, editing or removing a folder, or any JavaScript file in it, applies while Dupin runs. Settings → Extensions lists the loaded extensions with their keywords and a switch for each, and every folder it skipped with the reason. Turning one off is kept in `grants.toml` in Dupin's data directory (`$XDG_DATA_HOME/dupin`, else `~/Library/Application Support/Dupin`).

## Bundled extensions

Some extensions ship inside Dupin and run whether or not the extensions folder is on. The first is **Dupin** (`help`): type `help` and Space, then a word, for Dupin's documentation, issue tracker, source and website.

| Where                        | What                                                                                                                                                                                                                                  |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Dupin.app`                  | `Contents/Resources/extensions/<name>/`, read-only and inside the app's code signature.                                                                                                                                               |
| A build from the source tree | The repository's [`extensions/`](../../extensions), so `cargo run` and headless renders load them without packaging.                                                                                                                  |
| The repository               | Each package is TypeScript that Dupin runs as written, or JavaScript with `// @ts-check`, typed by [`sdk/dupin.d.ts`](../../sdk/dupin.d.ts). `checks.extensions-typecheck` runs `tsc --noEmit` over `extensions/tsconfig.json` in CI. |

Each build of Dupin records a hash of every file in each bundled package folder, and loads a folder there only while its files still match: a package added to the folder, or a file changed in it, stays off and is listed as skipped. A build from the source tree records the repository's `extensions/` as it was when it was built, so rebuild after editing one.

A bundled extension is on until you turn it off in Settings → Extensions. Dupin knows it by its `name` and a hash of its `package.json` and modules: while it runs exactly the copy that shipped, the [capability broker](../capability-broker.md) grants what it declares in advance, except files and programs you pick, and macOS still asks for its own permissions on first use ([Permissions](./permissions.md#extensions)). The broker keeps its grants and its switch under `bundled:<name>`, apart from any package in the extensions folder.

To change one, copy it into the extensions folder:

```sh
dupin ext eject dupin
```

`dupin ext eject NAME` copies the bundled package called `NAME` to `extensions/NAME` with files you can edit, and refuses if that folder exists. A package in the extensions folder with the same `name` as a bundled one replaces it while the extensions folder is on, and Settings → Extensions marks it as your copy. Once its `package.json` or a module differs from the bundled one by a byte, it is a third-party package with its own grants and switch: none of the bundled one's grants, consents or choices apply to it, and each capability it declares needs your consent. Deleting the folder, or turning the extensions folder off, brings back the bundled one with its grants. `dupin ext install` refuses a package named like a bundled one.

A user extension can't take a bundled extension's keyword, as it can't take a built-in command's: `dupin ext install` refuses it, and Settings lists one already in the folder as skipped and marks its row as not answering. Extensions from the folder load only once the bundled ones have been read, so one never holds a bundled keyword for a moment at launch.

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

| Key                  | Value                                                                                                                                                                         |
| -------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `name`               | Required. The extension's id: 1–64 lowercase letters, digits or hyphens, unique among extensions. No npm scopes.                                                              |
| `version`            | Required. 1–32 characters without spaces.                                                                                                                                     |
| `dupin.title`        | Required. Shown on the keyword chip and in Settings, up to 80 characters.                                                                                                     |
| `dupin.keyword`      | Required. Up to 32 bytes, no spaces.                                                                                                                                          |
| `dupin.main`         | Optional. The module that exports `search`: a `.js`, `.mjs`, `.ts`, `.mts` or `.tsx` file inside the folder, such as `dist/index.js` or `src/main.ts`. Defaults to `main.js`. |
| `dupin.capabilities` | Optional. The most the extension may ask for; see [Asking for more](#asking-for-more).                                                                                        |

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

Enter runs the first action: **Open URL** when the item has a `url`, otherwise **Copy**. → or ⌘K lists both. An item with neither shows but does nothing, and an item can't open a view of its own yet. Every value is checked; an unknown field, a value of the wrong type, an empty `text` or a URL with another scheme fails the whole answer with a row that says which item and field.

### What the module can reach

The module is plain modern JavaScript ([QuickJS-ng](https://quickjs-ng.github.io/quickjs/es_features/)): modules, `async`/`await`, regular expressions, `Date` and `JSON`, but no `Intl`. It can import other `.js` or `.mjs` files of its own folder with relative paths that include the extension (`import { rows } from "./lib/rows.js"`), also `.ts`, `.mts` and `.tsx` files when `dupin.main` is TypeScript, and the host module `dupin`, which for now exports only `apiVersion`. Nothing else resolves: no packages by name, no URLs, no absolute paths and no path or symlink that leads outside the folder.

There are no files, network, processes, clipboard, timers or console. Each query runs in a fresh engine, so module-level variables never carry from one query to the next. The engine runs in a separate, sandboxed process; see [Security model](#security-model).

### Asking for more

An extension asks for host services by declaring `capabilities` in the `dupin` field of its `package.json`. Dupin reads the declaration when it loads the package, rejects one it can't parse, and Settings → Extensions lists each scope it asks for and whether it is granted. The services themselves aren't connected to extensions yet; once they are, Dupin will check every call against that declaration and your grants:

```json
"capabilities": {
	"http": [{ "origin": "https://api.github.com", "methods": ["GET"] }],
	"files": { "notes": { "kind": "directory", "access": ["read", "list"] } },
	"exec": { "@script:open-in-zed": {} },
	"storage": { "quotaBytes": 1048576 },
	"clipboard": ["write"],
	"dictionary": ["lookup"],
	"calendar": { "access": ["read"], "pastDays": 0, "futureDays": 30 }
}
```

- A declaration is the most an extension can get. Anything malformed, unknown or out of range rejects it with a diagnostic naming the field.
- You grant each item separately, such as one origin or one named folder, and can revoke it at any time. Your grants belong to the version you approved: an update that asks for more asks again.
- Searching may use granted reads. Sending anything but GET or HEAD, running a program or script, and reading or writing the clipboard need you to choose the action.
- Calendar and clipboard reads also need macOS access, requested when you turn the extension on or first use it.

[Permissions](./permissions.md#extensions) has the rest, and [Capability broker](../capability-broker.md) the full rules.

The services behind the OS-backed capabilities are in place, ready for the broker bridge to call: each checks its inputs, reads macOS access without ever prompting, bounds what it returns and answers with JSON-shaped data or an error. An error has a `code`, a `message` and, for macOS access, the `permission` (`calendars` or `pasteboard`):

| `code`                  | Meaning                                                                                               |
| ----------------------- | ----------------------------------------------------------------------------------------------------- |
| `needs-permission`      | macOS hasn't been asked yet. An extension's call never asks; you do, from Dupin, with Request Access. |
| `permission-denied`     | macOS denied access; it changes in System Settings → Privacy & Security.                              |
| `permission-restricted` | A configuration profile or parental controls decide.                                                  |
| `needs-activation`      | The call needs you to choose the action.                                                              |
| `invalid`               | An argument is malformed or out of range.                                                             |
| `out-of-scope`          | The call reaches beyond what the extension declared.                                                  |
| `unsupported`           | This Mac can't do what was asked.                                                                     |
| `failed`                | macOS refused the operation.                                                                          |

### Dictionary

`"dictionary": ["lookup"]` lets an extension look a term up, while you type, in the dictionaries turned on in Dictionary's settings. It needs no macOS permission. A lookup takes a term of at most 200 characters and returns up to 8 entries, each with the structure Dupin's own `define` view lays out:

```json
{
	"headword": "run",
	"dictionary": null,
	"pronunciation": "rʌn",
	"groups": [
		{
			"partOfSpeech": "Verb",
			"note": "(runs, running; past ran; past participle run)",
			"senses": [
				{
					"definition": "[no object] move on foot faster than walking",
					"examples": ["she ran to the station"],
					"senses": [
						{
							"definition": "(of a machine) operate",
							"examples": ["the engine runs on diesel."],
							"senses": []
						}
					]
				}
			]
		}
	],
	"sections": [{ "title": "Origin", "text": "Old English rinnan." }],
	"truncated": false
}
```

Text the layout doesn't recognize stays in a sense's `definition` or a group's `note`, so nothing is lost. An entry keeps at most 32 groups, 64 senses each with 16 examples and one level of sub-senses, and 16 sections, each text cut to 4 KiB; `truncated` says whether anything was cut. macOS doesn't say which dictionary an entry came from, so `dictionary` is `null`, and naming dictionaries to look in is `unsupported`.

### Calendar

`"calendar": { "access": ["read"], "pastDays": 1, "futureDays": 30 }` lets an extension read events that fall within the declared number of whole days before and after today. A read names a range in JavaScript milliseconds and may name calendars, a query and a limit:

| Argument    | Bounds                                                                                                      |
| ----------- | ----------------------------------------------------------------------------------------------------------- |
| range       | `start` before `end`, both within the declared days; a wider range is `out-of-scope`, not quietly narrowed. |
| `calendars` | 1–32 calendar names. Leave it out for every calendar.                                                       |
| `query`     | At most 200 characters. Every word must appear in the title or location, ignoring case.                     |
| `limit`     | 1–500 events, 100 by default.                                                                               |

Events come earliest first:

```json
{
	"id": "7F3C…:1795431600000",
	"title": "Design review",
	"start": 1795431600000,
	"end": 1795435200000,
	"allDay": false,
	"calendar": { "name": "Work", "color": "#1badf8" },
	"location": "Meeting room",
	"url": null
}
```

`id` names one occurrence of a recurring event, `color` is the calendar's color in sRGB, and titles, locations and names are cut to 1 KiB. Reading needs macOS Calendars access. The service only checks it, and while macOS hasn't been asked a read returns `needs-permission`; an extension's call never asks. For now only Request Access on the Calendars row of Settings → Permissions asks macOS: turning the Extensions setting on doesn't, and nothing asks when you turn on one extension until the broker bridge connects these services.

### Clipboard

`"clipboard": ["read", "write", "watch"]` declares each kind of access separately.

- **Read** returns the current copy when you choose an action: `{ "change": 42, "content": … }`, or `null` when the clipboard is empty or the copy is one Dupin never reads.
- **Write** puts `{ "type": "text", "text": … }` (up to 1 MiB), `{ "type": "url", "url": … }` (an absolute URL of up to 8 KiB) or 1–64 files on the clipboard when you choose an action. Files are named by the handles that the extension's `files` reads and listings returned, never by path: an app you paste file references into acts on them as you, so an extension can only copy files inside a folder or file you granted it.
- **Watch** calls the extension with each new copy, the same shape as a read, without your choosing anything. Dupin's own clipboard monitor delivers the changes, so the extension never polls, and reads each copy once however many watches there are. At most 16 watches run at once; one that falls more than a few copies behind misses the newer ones rather than holding up clipboard history or other watches. Keeping a history is up to the extension, in its own `storage`.

A read's `content` is the most specific form the copy offers: files, then a URL, then an image (`{ "type": "image", "mediaType": "image/png", "data": "<base64>" }`, up to 2 MiB), then text. Like clipboard history, reads and watches skip copies that password managers mark concealed, transient or auto-generated, copies from the apps clipboard history ignores (`[clipboard] ignored_apps`), and text longer than its `max_entry_bytes`; Dupin checks those markers and apps before it reads the contents. Which app made a copy is only known if Dupin's clipboard monitor saw it land, by the app frontmost then, not when the extension reads it. So while `ignored_apps` lists any app, a read returns `null` for a copy made before the clipboard service started or within the half second before the read. Reading and watching need macOS to let Dupin paste from other apps without asking (**Always Allow**); under the default policy they return `needs-permission`, and Dupin only asks when you choose Request Access. Writing needs no macOS permission.

TypeScript whose types can simply be erased runs as written; see [TypeScript without a build](#typescript-without-a-build). JSX, TypeScript that needs compiling and npm packages are built on your machine: bundle the extension into one JavaScript module (`dupin ext build` does it with Rolldown; see [Authoring](#authoring)), point `dupin.main` at the output and install the folder. Dupin never type-checks and never runs a package manager or the package's scripts.

| Limit                        | Value                                                                                         |
| ---------------------------- | --------------------------------------------------------------------------------------------- |
| Modules (`.js` and `.ts`)    | 256 and 4 MiB                                                                                 |
| Heap                         | 16 MiB, or 64 MiB while compiling                                                             |
| Stack                        | 512 KiB                                                                                       |
| Time per query or compile    | 2 seconds                                                                                     |
| Promise jobs per query       | 10,000                                                                                        |
| Items and text in one answer | 50 and 256 KiB                                                                                |
| Extensions read              | 64 folders                                                                                    |
| Extension host memory        | 64 MiB plus twice the bytecode, or the compile heap and four times the source while compiling |

Typing again cancels the running query. A thrown error, a rejected or never-settling promise, an exhausted limit or an invalid answer becomes a "_Name_ failed" row with the file, line or field at fault; it never affects Dupin or other extensions. When Dupin loads or reloads the folder, the extension host compiles every module, so a syntax error, TypeScript that needs a build or a broken import is reported in Settings, with its file and line, before the first query.

### Security model

Dupin never parses, compiles or runs extension code itself; erasing TypeScript's types is part of the host's compile. The extension host is Dupin's own executable run with `--extension-host`. One host, started on the first query, runs every JavaScript extension's queries, and each package is compiled in a host started for that compile alone. Before it touches any extension code, the host confines itself with a deny-by-default macOS sandbox: it cannot open or list files, use the network, connect to other processes or system services, or start programs. Its resource limits allow no new files, child processes or core dumps. Dupin reads each package's files within the limits above and sends their text over a private socket; that host compiles them under the same deadline, limits and watchdog as a query, counted from when it is ready, returns bytecode or a diagnostic and exits. A compile never waits for another extension's query, and a host one package has subverted never compiles another. Dupin keeps the bytecode as opaque bytes, reuses it while the files' content is unchanged and sends it back with every query, so the host never reads the extensions folder and a package can never supply bytecode of its own. Dupin checks every answer again before showing it. The host keeps the time zone it starts with, so `Date` reports local time.

The sandbox contains an extension that breaks out of the engine through a flaw in QuickJS-ng, but because every JavaScript extension shares the host, such an extension could see the other extensions' code and queries and change their answers until the host is replaced. Install only extensions you trust. A host that sends anything Dupin didn't ask for is stopped.

The engine stops a query that runs past its deadline or is cancelled, but a few built-in operations, such as `indexOf` on very long strings, can't be interrupted, and the engine's heap limit doesn't bound the whole process. Dupin therefore watches the host from outside. It stops a host when a compile or query runs 150 ms past its deadline, ignores a cancellation for 150 ms or grows past its memory allowance, and when the host dies, the query becomes a "_Name_ failed" row. The next query starts a new host. One query runs at a time; the others wait in a bounded queue, and a query that typing replaces leaves it. The host exits when Dupin quits and after two minutes without queries, and bytecode Dupin kept is sent to the next host, so a restart doesn't recompile anything.

The sandbox exists on macOS only. On other platforms the host refuses to compile or run extensions, and Settings lists each JavaScript extension as failing with `sandbox-unavailable`; Linux needs its own confinement first. The host refuses for the same reason when Dupin itself already runs inside another sandbox.

### Testing one

Check and run a folder without the launcher:

```sh
dupin ext check examples/extensions/word-count-js
dupin ext run examples/extensions/word-count-js --query 'two words'
dupin ext run examples/extensions/word-count-js --query 'two words' --json
```

`check` loads the folder as the launcher does, has the extension host compile it, runs the module's top-level code and confirms it exports `search`. Both compile and run the module in the sandboxed extension host, as the launcher does, so a package behaves the same and is just as contained when you or an agent test it. `run` asks it one query and prints the rows and their actions, or with `--json` the answer in the [protocol](../plugin-protocol.md)'s `results` shape. Both exit non-zero on failure. With `--json`, a failure carries a diagnostic an agent can act on:

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

### `dupin ext new <dir> [--ts | --bundle]`

Creates a package in a new or empty folder. The folder's name becomes the id, so it takes 1–64 lowercase letters, digits or hyphens.

| File                          | Purpose                                                                                                     |
| ----------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `package.json`                | The name, version 0.1.0, `"keywords": ["dupin-extension"]` and the `dupin` key, with the id as the keyword. |
| `main.js` or `src/main.ts`    | A `search` that greets the query: `src/main.ts` with `--ts` or `--bundle`.                                  |
| `dupin.d.ts`                  | The API's declarations, for editors and `tsc --noEmit`.                                                     |
| `fixtures/search.json`        | A query and the items `search` answers it with.                                                             |
| `README.md`                   | How to develop, build and share the package.                                                                |
| `tsconfig.json`, `.gitignore` | `--ts` and `--bundle`. The compiler sees ES2023 and nothing else, as the guest does.                        |
| `rolldown.config.ts`          | `--bundle` only. How `dupin ext build`, `dupin ext dev` and the workflow bundle `src/main.ts`.              |
| `.github/workflows/build.yml` | `--bundle` only. Rebuilds `dist/main.js` with Rolldown and commits it on every push to `main`.              |

`dupin ext new .` names the package after the current folder.

With `--ts`, `dupin.main` is `src/main.ts`, which Dupin runs as written. With `--bundle`, for a package with npm dependencies, `package.json` adds `"source": "src/main.ts"`, pins `rolldown` to an exact version in `devDependencies`, and `dupin.main` is `dist/main.js`. Dupin ignores `source`; it tells `dupin ext build` what to bundle. Run `bun install` once and commit the `bun.lock` it writes.

The `--bundle` workflow installs exactly what `bun.lock` pins, with `--frozen-lockfile` and without install scripts, and runs Rolldown on Bun with the same `rolldown.config.ts`. It installs and bundles in a job that can only read the repository and keeps no Git credentials; a second job, which runs no dependency code, holds the write token and commits the bundle.

`dupin.d.ts` is a copy of [`sdk/dupin.d.ts`](../../sdk/dupin.d.ts) for the version of Dupin that made it. Its `apiVersion` matches the one the host's `dupin` module exports. Everything apiVersion 1 has is in the `dupin` module; what the [design](../design/extension-platform.md) plans next, such as `items(ctx)`, sections, Detail and Form views and the `http`, `files`, `exec`, `storage`, `clipboard`, `dictionary` and `calendar` capabilities, is drafted as types in its `v2` namespace. Nothing in `v2` works yet: an item with those fields fails the answer, and declared capabilities are checked and listed but give the extension nothing to call.

### TypeScript without a build

When `dupin.main` is a `.ts`, `.mts` or `.tsx` file, the package's TypeScript files are modules too, and the extension host erases their types as it compiles them: each annotation, interface, type alias, type argument and other type-only part becomes spaces of the same length, line breaks kept, so the line and column of every error and stack frame are the author's own and no source map is needed. Nothing is type-checked; run `tsc --noEmit` for that. Erasing a 2,000-line file takes about 1.3 ms.

Erased: type annotations and return types, `interface`, `type`, type parameters and arguments, `as`, `satisfies`, non-null `!`, `import type`, `export type` and `type` specifiers, `declare`, `abstract` classes and members, overload signatures, `?` and `!` on names, `public`, `private`, `protected`, `readonly` and `override` on class members, index signatures, `implements`, `this` parameters and namespaces that hold only types.

Rejected with `typescript-needs-build`, the file, line and column, and a hint: `enum` (`const` or not, and `declare const enum`, whose uses a compiler inlines), a namespace or `module` holding values, parameter properties (`constructor(private x)`), decorators, JSX, `import x = require(…)` or `import A = B.C`, and `export =`. Each needs a compiler to produce JavaScript; replace it as the hint says, or bundle the package. A `<T>value` assertion is rejected too, as `erasableSyntaxOnly` rejects it: blanking a type that ends its line after `return` would end the statement there. Write `value as T` instead. A module whose brackets nest more than 1,000 levels deep fails with `stack-limit` at the bracket that passes the limit.

- Import TypeScript files with their extension, `./util.ts`, as `allowImportingTsExtensions` lets `tsc` accept.
- Import types with `import type` or a `type` specifier. A type imported as a value is kept and fails to link, since nothing exports it; the scaffold's `verbatimModuleSyntax` and `erasableSyntaxOnly` make `tsc` and editors flag both mistakes.
- Declaration files (`.d.ts`, `.d.mts`) are not modules and are never compiled.
- With a JavaScript `dupin.main`, TypeScript files are ignored, so a bundled package can keep its sources beside the bundle.

Bundle with `dupin ext build` when the package uses npm packages, needs syntax that is rejected above, or should ship as one file.

### `dupin ext build <dir> [--sourcemap]`

A package with `source` is bundled into a single ES module at `dupin.main`, with its npm dependencies, by [Rolldown](https://rolldown.rs): the package's own `node_modules/.bin/rolldown`, or else `rolldown` on `PATH`. Dupin never downloads it; with neither, the build fails and says how to add it (`bun add -d rolldown` or `pnpm add -D rolldown`). Rolldown's CLI runs on Node.js.

The build uses the package's `rolldown.config.*`, which the `--bundle` scaffold writes, and a package without one is bundled with the same configuration built into Dupin. A `rolldown.config.js` or `.mjs` is refused, because the launcher loads every `.js` and `.mjs` file in the package as extension code; use `.ts`, `.mts`, `.cts` or `.cjs`. `dupin ext build` sets only the output file and, with `--sourcemap`, a linked `<main>.map`. The configuration produces one ES module for `platform: "neutral"`, with dynamic imports inlined and no other chunks, keeps `dupin` as the only external import, and reads each dependency's `module` or `main` field. The package is then checked with the launcher's own loader, the same check as `dupin ext check`: the manifest is validated in full, and the bundle is compiled and linked and its top-level code run.

A dependency that imports a Node built-in (`fs`, `net`, `child_process` and the rest, with or without `node:`), by `import`, `import()` or `require`, fails the build, naming the file that imports it. The configuration's resolver hook stops it before Rolldown would keep it as an external import, since the guest has no built-ins. Any other import Rolldown can't resolve, such as a dependency that isn't installed, fails the build too. A failed build or check puts back the `dupin.main` that was there before.

A package without `source` runs as written, JavaScript or TypeScript, and needs no build; `build` only checks it. [`dupin ext install`](#installing-and-updating) installs packages from Git without building them, so commit the bundle; the `--bundle` scaffold's workflow does that for you.

### `dupin ext dev <dir> [--sourcemap]`

Publishes a development copy to the extensions folder and keeps it current. It polls the package, skipping dot-files, `node_modules` and the bundle, and after each change it bundles (if the package has `source`), copies `package.json` and the modules into a new revision, and checks it as `dupin ext check` does. Only a revision that passes goes live: `extensions/<name>` is a symlink to `extensions/.dev/<name>/r<N>`, replaced in one rename, so a running Dupin with **Extensions folder** on reloads it and never sees a half-written copy. A failed build or check prints its diagnostic and leaves the previous revision live. Each revision is a line on stdout, or with `--json` a JSON object per line.

`dev` refuses to replace an `extensions/<name>` that is not its own symlink, so an installed extension of the same name is safe. Stop `dev` with ⌃C; the last revision stays installed until you delete `extensions/<name>` and `extensions/.dev/<name>`.

## Installing and updating

`dupin ext install` fetches a package and places it in the extensions folder, named after `name` in its `package.json`:

```sh
dupin ext install github:someone/dupin-word-count          # the newest version tag
dupin ext install github:someone/dupin-word-count@v1.2.0   # a tag or a branch
dupin ext install github:someone/dupin-word-count@0123456789abcdef0123456789abcdef01234567
dupin ext install https://git.example.com/me/notes.git@main # an https://, ssh:// or file:// URL, or user@host:path
dupin ext install ./word-count                              # a folder, copied as it is now
```

Without a ref, Dupin installs the highest version tag (`v1.2.3` or `1.2.3`; a pre-release ranks below its release, and `rc.10` above `rc.9`), or the default branch when the repository has no version tag. A commit is named by its full SHA. A URL can't carry a password or token, which would end up in `extensions.lock`; let Git supply it from a credential helper or an SSH key.

The installer reads the package as the [extension platform design](../design/extension-platform.md#packaging-and-distribution) describes it: the manifest is the `dupin` key of `package.json` (`title`, `keyword`, optionally `main` and `capabilities`), and the entry is committed to the repository: JavaScript or erasable TypeScript as written, or the bundle a build made.

Installing never runs anything from the package, and there is no package manager, install script or build. How Dupin fetches depends on the source:

- **`github:`** needs no developer tools. Dupin asks GitHub's REST API which commit the ref names, downloads GitHub's tarball of exactly that commit with the macOS system curl (`/usr/bin/curl`, ignoring `~/.curlrc`), and unpacks it itself. Every file must match the commit's tree, read from the API first: the same path, mode, size and Git blob id, so a package locks the same as it would from Git. A file that `.gitattributes` drops from archives (`export-ignore`) or rewrites in them (`export-subst`, line endings) fails the install, as does a Git LFS file when the repository's "Include Git LFS objects in archives" setting is on, because the archive then holds the object where the commit holds its pointer. If `GITHUB_TOKEN` is set, Dupin sends it to the API, for a private repository or a higher rate limit than GitHub's 60 requests an hour; it is never required, and never sent to the host the archive comes from.
- **A Git URL** runs `git` to fetch the one commit into a scratch repository, and Dupin reads that commit's files itself: nothing is checked out, so hooks, filters, Git LFS and submodules never run. It needs Git. Without it, the install fails with a message instead of running `/usr/bin/git`, which on a Mac without the Command Line Tools only opens their installer.
- **A folder** needs neither.

`dupin ext update` and `dupin ext sync` fetch the same way. Before placing anything, Dupin checks that:

- `package.json` has a valid `dupin` key and `name`;
- `dupin.main` (default `main.js`) is a `.js`, `.mjs`, `.ts`, `.mts` or `.tsx` file in the package, not a declaration file. A JSX entry, or a build output that isn't committed, fails with a hint to build and commit it;
- there are no symbolic links, submodules or `.git` folders, and no two names that differ only in case;
- there are at most 1,024 files, 4 MiB per file and 16 MiB in all, with paths up to 512 bytes and 16 folders deep. A fetch or download of more than 64 MiB is stopped before it finishes, and a tarball whose entries aren't plain files and folders inside its one top-level folder is refused;
- the keyword isn't already used by a built-in command, a web search keyword or another folder in the extensions folder;
- a folder with the package's name, if there is one, was installed by `dupin ext install` from the same source. Grants belong to that source, so installing another package under an installed name fails until you remove the old one.

A folder source skips `.git` and its top-level `node_modules`. The package is written into a hidden folder beside the others and then renamed into place, swapping with the previous copy on an update, so a running Dupin sees the old package or the new one and never half of either. It runs once the [extensions folder](#turning-them-on) is on, like any other folder there.

| Command                   | What it does                                                                                                                                                     |
| ------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `dupin ext list`          | Lists installed packages with their source, commit and state: installed, missing or modified.                                                                    |
| `dupin ext update [NAME]` | Moves one package, or all, to the newest version tag (or the default branch), or copies a folder source again, and shows the capability scopes it adds or drops. |
| `dupin ext remove NAME`   | Deletes the package's folder and its entry, with its grants.                                                                                                     |
| `dupin ext sync`          | Reinstalls every package that is missing or modified from its locked commit, and checks it has the locked content hash.                                          |

Each takes `--json`. Two commands that change extensions at the same time run one after the other.

### The lock file

`extensions.lock`, in Dupin's data directory (`$XDG_DATA_HOME/dupin`, else `~/Library/Application Support/Dupin`), pins what `dupin ext install` placed:

```toml
version = 1

[packages.word-count]
source = "github:someone/dupin-word-count"
ref = "v1.2.0"
commit = "0123456789abcdef0123456789abcdef01234567"
content_hash = "blake3:…" # every file's path, executable bit and bytes
installed_at = "2026-11-23T09:00:00Z"
version = "1.2.0"
keyword = "wc"
requested = ["http:https://api.example.com"]
granted = []
```

`requested` lists the scopes `dupin.capabilities` declares (see [Asking for more](#asking-for-more)), named as the [capability broker](../capability-broker.md) names grants: an `http` entry for `https://api.example.com` requests `http:https://api.example.com`, a `files` entry named `notes` requests `files:notes`, and `"clipboard": ["write"]` requests `clipboard:write`. A declaration the broker rejects stops the install. An update keeps the grants of scopes the package still requests and drops the rest. Scopes it adds go to `pending_consent` and stay ungranted until you agree; asking is the capability broker's job, which isn't connected yet.

When Dupin loads the extensions folder, it checks each locked package against its content hash, rehashing only after the package's files change. A package whose files differ, or that gained a symbolic link, is skipped and Settings → Extensions shows it as **disabled**; an unreadable lock keeps every extension off. `dupin ext sync` puts the installed copy back. Folders the lock doesn't list, such as ones you made yourself, aren't checked.

The lock is also how to reproduce a set of extensions: copy `extensions.lock` to another Mac and run `dupin ext sync`.

### Declaring them with Nix

The [Home Manager module](../configuration.md#nix) can declare packages and their grants instead:

```nix
programs.dupin = {
  enable = true;
  settings.extensions = true;
  extensions = [
    {
      src = pkgs.fetchFromGitHub {
        owner = "someone";
        repo = "dupin-word-count";
        rev = "0123456789abcdef0123456789abcdef01234567";
        hash = "sha256-…";
      };
      grants = [ "http:https://api.example.com" ];
    }
  ];
};
```

Each package links read-only into the extensions folder under `name`, which defaults to `name` in its `package.json`; set `name` to avoid reading the package while Nix evaluates. `grants` go to `extension-grants.toml` beside `config.toml`, the read-only base that grants chosen in Dupin overlay, as `settings.toml` overlays `config.toml`. macOS still asks for its own permissions when an extension first needs them. These packages are pinned by Nix, so they aren't in `extensions.lock`, and `dupin ext update` and `remove` leave them to Home Manager.

## Wasm extensions

Dupin no longer runs WebAssembly extensions. A folder that still holds an `extension.toml`, and no `package.json` with a `dupin` key, is skipped, and Settings → Extensions and `dupin ext check` say `extension.toml: Wasm extensions were removed; port to JavaScript`. A `package.json` that isn't valid JSON or passes 64 KiB reports its own error instead, so a port in progress points at the line to fix. To port one, write its `search` in JavaScript as [above](#a-javascript-extension), or start from `dupin ext new`; once `package.json` has a `dupin` key, a leftover `extension.toml` is ignored. Pushed views have no JavaScript equivalent yet.

## Cost

The JavaScript engine ([QuickJS-ng](https://quickjs-ng.github.io/quickjs/) through [`rquickjs`](https://github.com/DelSkayn/rquickjs)) is compiled into Dupin at `-O3`, which adds about 1 MiB to the signed app; the extension host is the same executable, so it adds no file. The extension host compiles a package to bytecode when Dupin loads the folder, so a query starts a fresh engine from bytecode and pays only for execution. On a Blacksmith M4 runner, a 400-candidate search returning 50 rows took 1.4 ms at the median and 1.8 ms at p95 over 2,000 queries to a running host, bytecode included. Compiling that package, which starts a host for the compile, took 11.7 ms at the median and 12.0 ms at p95; a first query from kept bytecode took 10.3 ms and 12.0 ms. Most of a start is the cost of starting Dupin's executable. The host's footprint after the warm queries was about 6.2 MiB. `cargo test --release -p dupin --test extension_host -- --ignored --nocapture` repeats the measurement.
