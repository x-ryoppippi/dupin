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

Dupin then reads `$XDG_CONFIG_HOME/dupin/extensions/` (or `~/.config/dupin/extensions/`). Each folder in it holds one extension. Adding, editing or removing a folder, or any JavaScript file in it, applies while Dupin runs. Settings → Extensions lists the loaded extensions with their keywords and a switch for each, and every folder it skipped with the reason. An extension from the folder answers its keyword once it loads but can use nothing it [asks for](#asking-for-more) until you turn it on; turning it off stops it answering. Your choice is kept in `grants.toml` in Dupin's data directory (`$XDG_DATA_HOME/dupin`, else `~/Library/Application Support/Dupin`).

## Bundled extensions

Some extensions ship inside Dupin and run whether or not the extensions folder is on. **Dupin** (`help`): type `help` and Space, then a word, for Dupin's documentation, issue tracker, source and website. **Emoji & Symbols** (`emoji`): pick an emoji or symbol by its English or Japanese name ([Emoji & Symbols](./emoji.md)). Some built-in features are bundled extensions too, such as [Calendar search](./calendar.md): each keeps its own setting and keywords, and Settings → Extensions lists it as a built-in command.

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

### Overriding a built-in

Every bundled extension follows the same rule, including one that answers a built-in command:

- **Copy it.** `dupin ext eject NAME` copies it to `extensions/NAME`. While the extensions folder is on, a package there with the bundled one's `name` answers in its place: under its keyword and in the main search, or, for a built-in command, under that command's keywords.
- **Unchanged, it is the bundled package.** It keeps the bundled grants and switch, and Settings → Extensions marks it as your unchanged copy.
- **Changed, it is your own package.** Once a byte of its `package.json` or a module differs, none of the bundled grants, consents or choices apply: it answers what you type but can use nothing it declares until you turn it on, and each capability then needs your consent. Settings → Extensions marks it as replacing the bundled one.
- **A built-in command keeps its setting and keywords.** The copy answers only while the command's own setting is on, under the keywords that setting gives, and the `keyword` in its `package.json` is ignored. A built-in command has two switches, both of which must be on: its setting registers the command, and the package's switch in Settings → Extensions lets the package run. Turning the package off leaves the command saying it isn't available; turning a copy off doesn't bring the bundled one back.
- **Delete it to go back.** Deleting the folder, or turning the extensions folder off, brings back the bundled one with its grants and switch as you left them.
- **Ejecting starts afresh.** `dupin ext eject` forgets what an earlier changed copy of the same name was allowed, turned on or stored, as `dupin ext install` does for a name installed afresh, so a new copy you change waits to be turned on again.

Settings → Extensions lists every bundled extension, built-in commands' included, with where it comes from (bundled, your unchanged copy, or a changed copy that replaces it), its switch and its grants, so each can be adjusted there. A built-in command's row says it answers that command in place of a keyword. The `dupin` crate's `extensions::overrides` test takes each package in `extensions/` through ejecting, answering, editing, its grants, turning the copy and the extensions folder off, deleting the copy and ejecting again, and fails for a package added there without a case of its own.

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

| Key                   | Value                                                                                                                                                                                                                |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `name`                | Required. The extension's id: 1–64 lowercase letters, digits or hyphens, unique among extensions. No npm scopes.                                                                                                     |
| `version`             | Required. 1–32 characters without spaces.                                                                                                                                                                            |
| `dupin.title`         | Required. Shown on the keyword chip and in Settings, up to 80 characters.                                                                                                                                            |
| `dupin.keyword`       | Required. Up to 32 bytes, no spaces.                                                                                                                                                                                 |
| `dupin.main`          | Optional. The module that exports `search` or `items`: a `.js`, `.mjs`, `.ts`, `.mts` or `.tsx` file inside the folder, such as `dist/index.js` or `src/main.ts`. Defaults to `main.js`.                             |
| `dupin.defaultSearch` | Optional, `false` unless set. `true` lists the package's [`items()`](#itemsctx) catalogue in the main search too, ranked with apps.                                                                                  |
| `dupin.emptyQuery`    | Optional, `false` unless set. `true` calls [`search`](#searchquery-ctx) with an empty query as soon as the keyword is typed, for a package with something to show before anything is typed, such as upcoming events. |
| `dupin.capabilities`  | Optional. What the extension may ask to use; see [Asking for more](#asking-for-more).                                                                                                                                |

Any other key inside `dupin` is an error. Every other key of `package.json`, such as `scripts`, `dependencies` or `devDependencies`, is ignored. A folder whose `package.json` has no `dupin` key is not an extension, and Settings says so. A keyword that another command already uses (`in`, a Dictionary alias, a web search keyword or another extension) is reported in Settings and the extension is skipped; extensions never take a keyword from anything else.

The module exports `search`, `items` or both. Each may be a function or an `async` function:

- **`search(query, ctx)`** answers each query itself, for what has to be computed from the query, such as a calculation or a lookup. It runs on every keystroke.
- **`items(ctx)`** hands Dupin a catalogue, such as bookmarks or clipboard history. Dupin keeps it and matches it as you type with the same matcher and usage ranking as apps, without running the package. It runs on first use, and again only after its `ttl` or when one of its actions or event handlers invalidates it.

A package with both lists what `search` computes for the query first, then the catalogue's matches.

### `search(query, ctx)`

`search` receives the text typed after the keyword and a frozen `ctx` whose `limit` is the number of rows Dupin will show. It returns an array of items, or a list: `{ type: "list", items: [...] }`, or `{ sections: [{ title, items }, ...] }` for items under headings (`type` may be left out). Return `[]` for no results. It isn't called before anything is typed, unless the manifest sets `emptyQuery`.

| Item field | Value                                                                                                                                                |
| ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `title`    | Required. One line, up to 512 bytes.                                                                                                                 |
| `subtitle` | Optional, up to 2,048 bytes.                                                                                                                         |
| `id`       | Unique among the items, up to 128 bytes. Optional for `search`, where Dupin numbers items without one; required for `items`.                         |
| `url`      | Optional. An `http` or `https` URL with a host, up to 2,048 bytes: **Open URL** opens it.                                                            |
| `text`     | Optional, up to 16 KiB: **Copy** copies it.                                                                                                          |
| `icon`     | Optional. An image in the package, relative to `package.json`, such as `assets/repo.png`: a `.png`, `.jpg`, `.jpeg`, `.svg`, `.pdf` or `.icns` file. |
| `section`  | Optional. The heading the item is listed under, one line up to 128 bytes. Items of a list's `sections` get their section's title.                    |
| `detail`   | Optional. `{ markdown, metadata }`, shown in place of the row while it is selected. See [Details](#details).                                         |
| `actions`  | Optional. More actions, after the ones `url` and `text` add; eight in all. See [Actions](#actions).                                                  |

Enter runs the first action: **Open URL** when the item has a `url`, then **Copy** when it has `text`, then its `actions` in order. → or ⌘K lists them all. An item without any shows but does nothing, and an item can't open a view of its own yet. Every value is checked; an unknown field, a value of the wrong type, an empty `text` or a URL with another scheme fails the whole answer with a row that says which item and field.

### `items(ctx)`

`items` returns the whole catalogue: an array of items, or a list with `items` or `sections` as for `search`. Its items may also carry `keywords`, up to 16 words of up to 64 bytes that find the item as well; a match on a keyword ranks just below the same match on the title, as an app's alias does.

```js
export function items() {
	return {
		ttl: 3600,
		sections: [
			{
				title: 'Projects',
				items: [
					{
						id: 'dupin',
						title: 'Dupin',
						keywords: ['launcher'],
						url: 'https://github.com/x-ryoppippi/dupin',
					},
				],
			},
		],
	};
}
```

Typing `bm` and Space lists the catalogue in its own order, under its sections. Typing more ranks every item by how well its title or keywords match, as apps are ranked, and puts the items you pick most often for that query first; the headings then give way to one ranked list. A list may also set:

| List field | Value                                                                                                                                      |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `ttl`      | Seconds, 1 to 86,400. Dupin asks again on the first use after they pass. Without it, the catalogue stays until it is invalidated.          |
| `cursor`   | Up to 256 bytes of the package's choosing, handed back as `ctx.cursor` the next time Dupin asks.                                           |
| `more`     | `true` when this is only the first part: Dupin asks again at once with `ctx.reason` `"more"` and the cursor, and keeps the parts together. |

`ctx` is frozen and holds `cursor`, the last answer's (`null` when Dupin holds nothing), and `reason`: `"load"` when Dupin holds no catalogue, `"more"`, `"expired"` after `ttl`, or `"invalidated"` after an action or event handler called `items.invalidate()`. One answer carries at most 500 items and 1 MiB of text, so a larger catalogue comes in parts with `more`. Every part, the first included, must arrive within the 2 seconds Dupin gives the search that asked, so parts make room for more items, not more time. Every catalogue item needs an `id` that stays with it as the list changes: Dupin applies updates by it and remembers which items you pick by it, so a position would hand one item's history to whatever takes its place.

Instead of the whole catalogue, an answer can change the one Dupin holds, by `id`, so a long and busy list such as clipboard history never sends everything again:

```js
export function items(ctx) {
	if (ctx.reason === 'load') return { items: everything(), cursor: latest() };
	return {
		type: 'update',
		append: since(ctx.cursor),
		remove: forgotten(ctx.cursor),
		cursor: latest(),
	};
}
```

`append` adds items Dupin doesn't hold after the others, `replace` swaps the items with those IDs in place, and `remove` lists IDs to drop; an ID Dupin doesn't hold is ignored. An update that appends an ID Dupin already holds, or replaces one it doesn't, is refused whole: Dupin shows the failure, drops the catalogue and asks again from the start, with `reason` `"load"` and no cursor. So does any failure of `items`.

Dupin holds each catalogue in memory, at most 10,000 items and 8 MiB of text, and 24 MiB across all extensions; past that, the least recently used catalogue is dropped and asked for again when next used. Reloading the package, including any edit to its files, starts an empty catalogue. Nothing is written to disk: a catalogue costs one call to rebuild after Dupin starts, and keeping it would leave what a package lists, such as clipboard history, on disk beyond the package's say; a list that must outlast a restart lives in the package's `storage`, which `items` reads.

`items` reaches host services as `search` does. It runs while you type, never because you chose an action, so it may use what you allowed that needs no action of yours (files, GET and HEAD, the dictionary, calendars and `storage`, whose writes and deletes need none either), and anything that needs an action you chose, such as reading the clipboard, fails with `needs-activation`. A request that needs consent shows its question under the keyword. A catalogue belongs to the access it was read under: revoking a scope or turning the extension off drops it, allowing a scope drops it too, and either way the next use asks for it from the start with `reason` `"load"`.

#### The main search

With `"defaultSearch": true` in its manifest, a package's catalogue also answers the main search, after apps and in the order of the extension folders, with up to eight rows ranked by the same matcher. Dupin matches the catalogue it holds, so the package never sees what you type there and `search` is never called for it. Loading the catalogue from the main search reaches host services as it does under the keyword, but the main search never asks for consent: a catalogue that fails to load, for want of consent or otherwise, stays out of it and shows its failure, and any question, under its own keyword. The main search doesn't ask a failed catalogue again until its access changes, something invalidates it or its keyword loads it, so it costs no call per keystroke.

### Details

An item's `detail` is shown in place of its row while it is selected, as Dictionary's entries are:

| Detail field | Value                                                                                                                                                                             |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `markdown`   | Optional, up to 32 KiB. Headings, paragraphs, bold, italic, lists, inline and fenced code and `http` or `https` links; Dupin opens a link when you press it. Raw HTML stays text. |
| `metadata`   | Optional. Up to 12 `{ label, text }` entries shown above the Markdown, each one line: `label` up to 64 bytes and `text` up to 512.                                                |

The detail's title is the item's.

### Actions

An item's `actions` adds up to eight actions, counting the ones `url` and `text` make. Each has a one-line `title` of up to 512 bytes and exactly one of:

| Action  | What it does                                                                                                                                                                                                                                  |
| ------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `copy`  | Copies the text, 1 byte to 16 KiB.                                                                                                                                                                                                            |
| `paste` | Pastes the text, 1 byte to 16 KiB, into the app in front of Dupin once the panel closes. Only a package that [declares](#clipboard) `"clipboard": ["paste"]`, and that you allow to, may paste; otherwise Dupin copies the text and says why. |
| `open`  | Opens the `http` or `https` URL, or a [calendar event's `link`](#calendar).                                                                                                                                                                   |
| `run`   | Calls `actions[run](data, ctx)` in the package: `run` names one of its functions, up to 64 letters, digits, `_` or `$`, and `data` is up to 4 KiB of JSON, nested 16 levels deep.                                                             |

```js
import { items as catalogue, storage } from 'dupin';

export async function search(query) {
	const { records } = await storage.list({ limit: 20 });
	return [
		{ title: `Save “${query}”`, actions: [{ title: 'Save', run: 'save', data: { text: query } }] },
		...records.map((note) => ({ title: note.data, text: note.data })),
	];
}

export const actions = {
	async save({ text }) {
		await storage.put(text);
		return { type: 'toast', text: 'Saved' };
	},
	forget(data) {
		forgetEntry(data.id);
		catalogue.invalidate();
	},
};
```

A `run` action keeps the launcher open. The function receives `data`, `null` when the action has none, and a frozen, empty `ctx`, and may return nothing or `{ type: "toast", text }`, one line of up to 512 bytes that Dupin shows in the panel. It runs in a fresh engine like a query, but for up to 30 seconds and in a host of its own, so a slow action never holds up typing, and only an action you chose may write, send, run a program or touch the clipboard; see [Asking for more](#asking-for-more). Only the extension whose row offered an action runs it, and only own properties of `actions` are handlers, so `toString` names nothing. Calling `items.invalidate()` from `dupin` there, or in an [event handler](#events), asks `items` again, with its cursor and `reason` `"invalidated"`; called from `search` or `items`, it throws. Dupin searches the current query again once an action ends, so the change shows at once. A handler that throws, or names an export the package doesn't have, shows its failure as a notice.

### What the module can reach

The module is plain modern JavaScript ([QuickJS-ng](https://quickjs-ng.github.io/quickjs/es_features/)): modules, `async`/`await`, regular expressions, `Date` and `JSON`, but no `Intl`. It can import other `.js` or `.mjs` files of its own folder with relative paths that include the extension (`import { rows } from "./lib/rows.js"`), also `.ts`, `.mts` and `.tsx` files when `dupin.main` is TypeScript, and the host module `dupin`. Nothing else resolves: no packages by name, no URLs, no absolute paths and no path or symlink that leads outside the folder.

There are no files, network, processes, clipboard, timers or console of the engine's own. The `dupin` module exports `apiVersion` (`2`), `DupinError`, `items`, whose `invalidate()` an action or event handler calls, and one object per capability, `http`, `files`, `exec`, `storage`, `clipboard`, `dictionary` and `calendar`, whose functions return promises; each works only for what the package [declares](#asking-for-more) and you allow, from `search`, `items`, actions and events alike. Each query, `items` call, action and event runs in a fresh engine, so module-level variables never carry from one call to the next; `cursor` is how `items` knows where it left off, and `storage` keeps what else must last. The engine runs in a separate, sandboxed process; see [Security model](#security-model).

### Events

The `events` export receives what Dupin watches for the package. `events.clipboard(change)` gets each new copy while the package is on and holds `clipboard:watch`:

```js
import { storage } from 'dupin';

export const events = {
	async clipboard(change) {
		if (change.type !== 'text') return;
		await storage.put({ text: change.text });
	},
};
```

`change` is frozen and shaped as `clipboard.read()` returns a copy, with text cut to 64 KiB and `truncated: true` when it was. One delivery runs at a time for each package; copies that land meanwhile are coalesced to the newest, so a slow handler never builds a queue. A handler runs in the background: it may read and use `storage`, but anything that needs an action you chose fails with `needs-activation`. Revoking the scope, turning the extension off or macOS withdrawing access stops the deliveries at once.

A handler that calls `items.invalidate()` has Dupin ask `items` again, with its cursor and `reason` `"invalidated"`, and, while the panel lists that catalogue under the keyword or in the main search, search the current query again so the change shows at once. Clipboard history is one catalogue kept this way: each copy is stored, the cursor is the newest record's ID, and `items` sends only the records listed before it, or everything again when it is gone or the catalogue expired, which also drops records storage has since deleted:

```js
import { items as catalogue, storage } from 'dupin';

const row = (record) => ({ id: record.id, title: record.data.text, text: record.data.text });

export async function items(ctx) {
	const { records } = await storage.list({ limit: 200 });
	const seen = records.findIndex((record) => record.id === ctx.cursor);
	const cursor = records[0]?.id;
	if (ctx.reason !== 'invalidated' || seen < 0) {
		return { items: [...records].reverse().map(row), cursor, ttl: 3600 };
	}
	return { type: 'update', append: records.slice(0, seen).reverse().map(row), cursor, ttl: 3600 };
}

export const events = {
	async clipboard(change) {
		if (change.type !== 'text') return;
		await storage.put({ text: change.text });
		catalogue.invalidate();
	},
};
```

### Asking for more

An extension asks for host services by declaring `capabilities` in the `dupin` key of its `package.json`. Dupin checks every request against that declaration, the version you allowed and your choices:

```json
"capabilities": {
	"http": [{ "origin": "https://api.github.com", "methods": ["GET"] }],
	"files": { "notes": { "kind": "directory", "access": ["read", "list"] } },
	"exec": { "git-status": { "args": ["status", "--short", "{path}"], "output": true }, "@script:open-in-zed": {} },
	"storage": { "quotaBytes": 1048576, "retentionDays": 30 },
	"clipboard": ["read", "write", "watch", "paste"],
	"dictionary": ["lookup"],
	"calendar": { "access": ["read"], "pastDays": 0, "futureDays": 30 }
}
```

- A declaration is the most an extension can get. Anything malformed, unknown or out of range rejects the package with a diagnostic naming the field, such as `dupin.capabilities.http[0].origin`.
- Each item is a scope you allow on its own: one origin, one named folder, one profile. A scope's name, such as `http:https://api.github.com`, `files:notes`, `exec:git-status`, `script:open-in-zed`, `storage`, `clipboard:read`, `dictionary` or `calendar`, is what errors, Settings and grant files call it.
- An extension that declares capabilities starts off. Its first request while off fails with `disabled`, and the panel offers **Turn on _Name_?**. Turning it on grants nothing.
- A request for a scope you haven't decided on fails with `needs-consent`, and the panel shows **Allow _Name_ to …?** above the extension's rows, with **Allow** and **Deny**, until you answer. Settings → Extensions lists the same requests, and every scope each extension declares with whether it is allowed, with **Allow**, **Revoke** and **Request Access**. A files or exec scope is allowed from Settings, where you pick the file, folder or program. Allowing runs nothing the extension asked for before; use it again. **Deny** revokes the scope until you allow it in Settings.
- Your choices belong to the version you allowed: a change to `package.json` or any module is another revision. A revision that declares a scope no wider keeps your choice; one that widens it asks again; one that drops it forgets it.
- While you type, an extension may use what you allowed to read: GET and HEAD, files, the dictionary and calendars, and its own `storage`, writes included. Any other HTTP method, a program or script and reading, writing or pasting from the clipboard need an action you chose. Dupin, not the extension, says which: a query, an event or an action of yours.
- Revoking a scope or turning an extension off takes effect at once: a request in flight finishes, but its result is dropped, the query's rows are dropped with it, and allowing again doesn't bring them back.
- Calendars, reading or watching the clipboard, and pasting also need macOS access. Dupin never asks macOS on its own: not at startup, on opening Settings, on reload or while an extension runs. A request that needs it fails with `needs-consent` and `permission` set, and **Request Access** in the panel or Settings asks macOS once.

[Permissions](./permissions.md#extensions) and [Capability broker](../capability-broker.md) have the full rules.

A request that fails rejects with a `DupinError` whose `code` the extension can branch on; `scope`, `description` and `permission` say what consent is missing:

```js
import { clipboard, DupinError } from 'dupin';

export const actions = {
	async paste() {
		try {
			return { type: 'toast', text: (await clipboard.readText()) ?? 'Nothing to paste' };
		} catch (error) {
			if (error instanceof DupinError && error.code === 'needs-consent') {
				return { type: 'toast', text: `Allow ${error.description} first` };
			}
			throw error;
		}
	},
};
```

| `code`             | Meaning                                                                                                   |
| ------------------ | --------------------------------------------------------------------------------------------------------- |
| `needs-consent`    | You haven't allowed the scope, an update widened it, or macOS hasn't been asked. Dupin shows the request. |
| `needs-activation` | Only an action you chose may do this.                                                                     |
| `disabled`         | The extension is off.                                                                                     |
| `revoked`          | You revoked the scope, or its access changed while the request ran and the result was dropped.            |
| `denied`           | Your declared configuration denies the scope.                                                             |
| `os-denied`        | macOS denied or restricts the access; it changes in System Settings → Privacy & Security.                 |
| `undeclared`       | The package doesn't declare the scope, such as an origin it didn't list.                                  |
| `out-of-scope`     | The request goes beyond the declaration: another method, a files access or a wider calendar window.       |
| `quota-exceeded`   | A storage write would pass the declared quota.                                                            |
| `invalid`          | An argument is malformed or out of range.                                                                 |
| `too-many-calls`   | More than 16 requests wait at once, or one call made more than 256.                                       |
| `too-large`        | A request or its result is larger than Dupin passes (2 MiB out, 4 MiB back).                              |
| `unavailable`      | No service answers here, as in `dupin ext run`, or this build has none for the capability yet.            |
| a service's code   | Such as `not-found`, `timeout`, `blocked-address` or `failed`, with the service's message.                |

A request counts against its call's deadline: 2 seconds while you type, 30 seconds for an action.

### HTTP

`"http": [{ "origin": "https://api.github.com", "methods": ["GET"] }]` declares up to 32 HTTPS origins, each with the methods it may use. `http.fetch(url, { method, headers, body })` returns a response with `ok`, `status`, `url`, `redirected`, `headers` (lowercase names, without cookies), `truncated`, `text()` and `json()`:

```js
import { http } from 'dupin';

export async function search(query) {
	const response = await http.fetch(
		`https://api.github.com/search/repositories?q=${encodeURIComponent(query)}`,
		{
			headers: { accept: 'application/vnd.github+json' },
		},
	);
	if (!response.ok) throw new Error(`GitHub returned ${response.status}`);
	const { items } = await response.json();
	return items.slice(0, 10).map((repo) => ({ title: repo.full_name, url: repo.html_url }));
}
```

Dupin sends no cookies or credentials of its own, connects only to the permit's origin, follows a redirect only within it, refuses private and reserved addresses, and caps a request at 64 headers and 1 MiB of body and a response at 2 MiB, past which `truncated` is set.

### Files

`"files": { "notes": { "kind": "directory", "access": ["read", "list"] } }` declares named scopes; you pick the file or folder each one is when you allow it in Settings. `files.list(scope, path?)` returns entries with `name`, `kind`, `size` and a `handle`; `files.readText(scope, path)` the text of a file, and `files.read` also its `size` and whether it was `truncated`. A files scope's name is lowercase letters, digits, `-` and `_`. Paths are relative to the scope with `/`, and the [files capability](../extension-capabilities.md#files) describes how they are checked.

```js
import { files } from 'dupin';

export async function search(query) {
	const notes = await files.list('notes');
	return notes
		.filter((entry) => entry.kind === 'file' && entry.name.includes(query))
		.map((entry) => ({ title: entry.name }));
}
```

### Exec

`"exec"` declares named profiles, each an argument template whose `{input}` placeholders take one line of text, or `@script:<name>` for one of your [script commands](./scripts.md). You pick the program a profile runs when you allow it; a script needs no pick. `output: true` asks to receive stdout and stderr. `exec.run(profile, inputs)` runs only in an action and returns `code`, `signal`, `timedOut` and, with output allowed, `stdout`, `stderr` and `truncated`:

```js
import { exec } from 'dupin';

export const actions = {
	async status({ path }) {
		const result = await exec.run('git-status', { path });
		return {
			type: 'toast',
			text: result.code === 0 ? result.stdout.trim() || 'Clean' : `git exited ${result.code}`,
		};
	},
};
```

Programs and scripts run with your own access, without a shell, as the [exec capability](../extension-capabilities.md#exec) describes; consent shows the program and every argument.

### Storage

`"storage": { "quotaBytes": 1048576, "retentionDays": 30 }` gives the package an encrypted store of its own: records of JSON, each with the blobs (byte strings, such as an image) it carries. It works while you type, in actions and in event handlers, once you allow it.

```js
import { storage } from 'dupin';

const thumbnail = new Uint8Array([0x89, 0x50, 0x4e, 0x47]);
const record = await storage.put({ title: 'Cover' }, { blobs: [thumbnail] });
const { records, next } = await storage.list({ limit: 20 });
const bytes = await storage.readBlob(record.blobs[0].handle);
```

| Function                  | What it does                                                                                                                                                                                         |
| ------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `put(data, { blobs })`    | Stores a record and returns it: `{ id, createdAt, data, blobs: [{ handle, size }] }`. Blobs are typed arrays or `ArrayBuffer`s.                                                                      |
| `get(id)`                 | The record, or `undefined`.                                                                                                                                                                          |
| `list({ before, limit })` | Up to `limit` records (50 by default, 200 at most) older than the record `before` names, newest first, and `next` to pass as `before` for the following page. A page also ends before 2 MiB of JSON. |
| `readBlob(handle)`        | The blob's bytes as a `Uint8Array`, while a record still carries it.                                                                                                                                 |
| `delete(id)`, `clear()`   | Delete one record, or all of them. A blob goes with the last record that carries it.                                                                                                                 |
| `usage()`                 | `{ entries, bytes, quotaBytes, retentionDays }`.                                                                                                                                                     |

Record IDs and blob handles are opaque strings. IDs are random, so they don't sort by when the record was stored; `list` gives that order. The same blob stored twice is kept once and counts once. Writes, reads and deletes all need the grant; none needs an action you chose.

- **Limits.** A record's JSON and its blobs are at most 1 MiB together, with at most 16 blobs; a larger one fails with `too-large`. The package keeps up to 10,000 records and `quotaBytes` (at most 64 MiB) in all. A `put` that doesn't fit fails with `quota-exceeded` and deletes nothing; Dupin checks the room and writes in one step, so writes made at the same time can't pass the quota together. Records older than `retentionDays` (1–3,650) are deleted; without it they stay until the package deletes them. An update that lowers the quota or the retention deletes the oldest records that no longer fit.
- **Where it lives.** Each package has a [`dupin-storage`](./storage.md) namespace of its own, named by a hash of its package ID and where it was installed from, so no package reads another's data, a native feature's, or that of a bundled package it copies. Everything is sealed with the data key in your Keychain.
- **Revoking keeps the data, removing deletes it.** Revoking `storage` in Settings or the panel stops every request at once and keeps what the package stored, so allowing it again finds it; Settings → Extensions offers **Delete Data** beside the scope. `dupin ext remove` deletes the package's data with it, asking the running Dupin to if it has the data open. The namespace follows the package name, so `dupin ext install` of a name that isn't installed also deletes what an earlier package of that name left behind, and doesn't install while that data can't be deleted.

### Dictionary

`"dictionary": ["lookup"]` lets an extension look a term up, while you type, in the dictionaries turned on in Dictionary's settings. It needs no macOS permission. `dictionary.lookup(term)` takes a term of at most 200 characters and returns up to 8 entries, each with the structure Dupin's own `define` view lays out:

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

Text the layout doesn't recognize stays in a sense's `definition` or a group's `note`, so nothing is lost. An entry keeps at most 32 groups, 64 senses each with 16 examples and one level of sub-senses, and 16 sections, each text cut to 4 KiB; `truncated` says whether anything was cut. macOS doesn't say which dictionary an entry came from, so `dictionary` is `null`.

### Calendar

`"calendar": { "access": ["read"], "pastDays": 1, "futureDays": 30 }` lets an extension read events that fall within the declared number of whole days before and after today. `calendar.events({ from, to, query, limit, order })` takes a `Date`, an ISO 8601 string or JavaScript milliseconds for each end, with `to` exclusive, and up to 500 events, 100 by default; a range wider than the declared days is `out-of-scope`, not quietly narrowed. With `query`, up to 200 characters, only events whose title or location holds every word of it, in any order and case, come back, so a search sends the extension only what matches:

```js
import { calendar } from 'dupin';

export async function search() {
	const now = new Date();
	const tomorrow = new Date(now.getTime() + 864e5);
	const events = await calendar.events({ from: now, to: tomorrow });
	return events.map((event) => ({
		title: event.title,
		subtitle: `${event.start} · ${event.calendar.name}`,
	}));
}
```

Events come earliest start first, and the limit keeps the earliest. With `order: 'latest'` they come latest start first and the limit keeps the latest, which is what a read of past events wants. `start` and `end` are ISO 8601 in UTC:

```json
{
	"id": "7F3C…:1795431600000",
	"title": "Design review",
	"start": "2026-11-24T10:00:00.000Z",
	"end": "2026-11-24T11:00:00.000Z",
	"allDay": false,
	"calendar": { "name": "Work", "color": "#1badf8" },
	"location": "Meeting room",
	"url": null,
	"link": "ical://ekevent/20261124T100000Z/7F3C…?method=show&options=more",
	"labels": {
		"day": "Tuesday 24 November",
		"time": "10:00 – 11:00",
		"dates": "24 Nov, 10:00 – 11:00"
	}
}
```

`id` names one occurrence of a recurring event, `color` is the calendar's color in sRGB, a title comes as one line of up to 512 bytes, so it fits a row's `title` as it is, and locations and names are cut to 1 KiB. `url` is the event's own link, if its calendar holds one. `link` shows the occurrence in Calendar: give it as an [`open` action](#actions) of the `search` that read it, the only scheme besides `http` and `https` one opens. A link no read in that search returned, or one in an `items()` catalogue, fails the answer with `invalid-url`. The guest has no `Intl`, so `labels` has the event's dates as this Mac's locale and time zone write them: `day` the weekday and date it starts, `time` only its time, or `All day`, when it falls within one day and `null` otherwise, and `dates` its dates and times, each with the year when it isn't this year. Reading also needs macOS Calendars access, which only **Request Access** asks for. The bundled [Calendar search](./calendar.md) is written this way.

### Clipboard

`"clipboard": ["read", "write", "watch", "paste"]` declares each kind of access separately.

- **Read**: `clipboard.read()` returns the current copy when you choose an action, or `null` when the clipboard is empty or the copy is one Dupin never reads; `clipboard.readText()` returns its text, if it is text.
- **Write**: `clipboard.writeText(text)` puts up to 1 MiB of text on the clipboard when you choose an action.
- **Paste**: a row's [`paste` action](#actions) puts its text in the app in front of Dupin when you choose it. Dupin presses ⌘V for it, which needs macOS Accessibility access; until both are allowed, choosing it copies the text, shows why, and leaves the request in the panel and Settings. The extension's code never pastes: the action is data, and `clipboard:paste` only lets Dupin carry it out.
- **Watch**: [`events.clipboard`](#events) receives each new copy, without your choosing anything. Dupin's own clipboard monitor delivers the changes, so the extension never polls, and reads each copy once however many watches there are; at most 16 run at once. Keeping a history is up to the extension, in its own `storage`.

A copy is the most specific form it offers: `{ "type": "files", "files": [{ "path", "name" }] }`, then `{ "type": "url", "url" }`, then an image (`{ "type": "image", "mediaType": "image/png", "data": "<base64>" }`, up to 2 MiB), then `{ "type": "text", "text" }`. Like clipboard history, reads and watches skip copies that password managers mark concealed, transient or auto-generated, copies from the apps clipboard history ignores (`[clipboard] ignored_apps`, followed from the next copy when it changes), and text longer than its `max_entry_bytes`; Dupin checks those markers and apps before it reads the contents. Which app made a copy is only known if Dupin's clipboard monitor saw it land, by the app frontmost then, so while `ignored_apps` lists any app, a read returns `null` for a copy made before the clipboard service started or within the half second before the read. Reading and watching need macOS to let Dupin paste from other apps without asking (**Always Allow**); writing needs no macOS permission.

TypeScript whose types can simply be erased runs as written; see [TypeScript without a build](#typescript-without-a-build). JSX, TypeScript that needs compiling and npm packages are built on your machine: bundle the extension into one JavaScript module (`dupin ext build` does it with Rolldown; see [Authoring](#authoring)), point `dupin.main` at the output and install the folder. Dupin never type-checks and never runs a package manager or the package's scripts.

| Limit                                           | Value                                                                                         |
| ----------------------------------------------- | --------------------------------------------------------------------------------------------- |
| Modules (`.js` and `.ts`)                       | 256 and 4 MiB                                                                                 |
| Heap                                            | 16 MiB, or 64 MiB while compiling                                                             |
| Stack                                           | 512 KiB                                                                                       |
| Time per query, `items` load, action or compile | 2 seconds                                                                                     |
| Promise jobs per call                           | 10,000                                                                                        |
| Items and text in one answer                    | 50 and 256 KiB; for `items`, 500 and 1 MiB                                                    |
| A catalogue                                     | 10,000 items and 8 MiB of text; 24 MiB across all extensions                                  |
| Extensions read                                 | 64 folders                                                                                    |
| Extension host memory                           | 64 MiB plus twice the bytecode, or the compile heap and four times the source while compiling |

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

`check` loads the folder as the launcher does, has the extension host compile it, runs the module's top-level code and confirms it exports `search` or `items`, and says which. Both compile and run the module in the sandboxed extension host, as the launcher does, so a package behaves the same and is just as contained when you or an agent test it. `run` answers one query as the launcher would, from `search` and the `items` catalogue, and prints the rows under their sections with their actions, or with `--json` the answer in the [protocol](../plugin-protocol.md)'s `results` shape, each row with its `section`, `icon` and `detail` beside it and a `run` action's effect as `{"kind": "run", "handler", "data"}`. An empty `--query` lists the catalogue. Both exit non-zero on failure. With `--json`, a failure carries a diagnostic an agent can act on:

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

To replay several steps against stated expectations, with host services answered from the fixture instead of this Mac, or to see the panel a package produces, use [`dupin ext test` and `dupin ext snap`](#testing-and-snapshots).

## Authoring

`dupin ext` has five commands for writing an extension. They describe a package with `package.json`, whose `dupin` key is the manifest: `title`, `keyword` and optionally `main`, the module Dupin runs, which defaults to `main.js`. The package's `name` is its id and the name of its folder in `extensions/`.

### `dupin ext new <dir> [--ts | --bundle]`

Creates a package in a new or empty folder. The folder's name becomes the id, so it takes 1–64 lowercase letters, digits or hyphens.

| File                          | Purpose                                                                                                     |
| ----------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `package.json`                | The name, version 0.1.0, `"keywords": ["dupin-extension"]` and the `dupin` key, with the id as the keyword. |
| `main.js` or `src/main.ts`    | A `search` that greets the query: `src/main.ts` with `--ts` or `--bundle`.                                  |
| `dupin.d.ts`                  | The API's declarations, for editors and `tsc --noEmit`.                                                     |
| `fixtures/search.json`        | A [fixture](#fixtures): a query, the row it answers with and what Enter on it does.                         |
| `README.md`                   | How to develop, build and share the package.                                                                |
| `tsconfig.json`, `.gitignore` | `--ts` and `--bundle`. The compiler sees ES2023 and nothing else, as the guest does.                        |
| `rolldown.config.ts`          | `--bundle` only. How `dupin ext build`, `dupin ext dev` and the workflow bundle `src/main.ts`.              |
| `.github/workflows/build.yml` | `--bundle` only. Rebuilds `dist/main.js` with Rolldown and commits it on every push to `main`.              |

`dupin ext new .` names the package after the current folder.

With `--ts`, `dupin.main` is `src/main.ts`, which Dupin runs as written. With `--bundle`, for a package with npm dependencies, `package.json` adds `"source": "src/main.ts"`, pins `rolldown` to an exact version in `devDependencies`, and `dupin.main` is `dist/main.js`. Dupin ignores `source`; it tells `dupin ext build` what to bundle. Run `bun install` once and commit the `bun.lock` it writes.

The `--bundle` workflow installs exactly what `bun.lock` pins, with `--frozen-lockfile` and without install scripts, and runs Rolldown on Bun with the same `rolldown.config.ts`. It installs and bundles in a job that can only read the repository and keeps no Git credentials; a second job, which runs no dependency code, holds the write token and commits the bundle.

`dupin.d.ts` is a copy of [`sdk/dupin.d.ts`](../../sdk/dupin.d.ts) for the version of Dupin that made it. Its `apiVersion` matches the one the host's `dupin` module exports. Everything apiVersion 2 has is in the `dupin` module: `search`, `items(ctx)`, sections, details, item actions, `actions` and `events`, the capabilities and `DupinError`. What the [design](../design/extension-platform.md) plans next, such as accessories and pushed Detail and Form views, is drafted as types in its `planned` namespace. Nothing in `planned` works yet: an item with those fields fails the answer.

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

### Testing and snapshots

```sh
dupin ext test greeter
dupin ext test greeter --update
dupin ext snap greeter greeter/fixtures/search.json search.png
```

`dupin ext test <dir> [--update] [--json]` replays every [fixture](#fixtures) in the package's `fixtures/` folder. It loads and compiles the package as the launcher does, then runs each step through the sandboxed extension host, with host requests going through the broker as they would in the launcher, and compares what each step produced with the step's `expect`:

```text
FAIL fixtures/search.json
  step 1, query "Dupin":
    rows[0].title: expected "Hello, Dupin!", got "Hi, Dupin!"
1 fixture: 0 passed, 1 failed
```

Each difference names the path in the outcome and both values; something the outcome has that the expectation doesn't name is `unexpected`, and something it lacks is `got nothing`. A step that can't run, such as selecting a row the answer doesn't have, stops its fixture. `test` exits non-zero when any fixture fails or can't be read. `--json` prints `{ "package", "passed", "fixtures": [{ "file", "passed", "updated", "steps": [{ "step", "action", "passed", "outcome", "differences", "problem" }] }] }`, and a fixture that can't be read has a `diagnostic` instead of `steps`. `--update` rewrites each fixture whose steps all ran so every step expects what it produced, and passes it; a fixture is rewritten only when that changes it.

`dupin ext snap <dir> <fixture> <output.png> [--step N] [--json]` renders the panel after step `N`, counting from 1, or after the last step. It starts this Dupin headless in a scratch home that holds only a copy of the package's `package.json` and modules, with **Extensions** on and nothing else from your configuration, history or Mac, types each query after the keyword and moves the selection as the steps do, and saves the panel as Dupin draws it. Activations and events aren't replayed, so the panel is the one the action was chosen from; to show what they would have stored, seed `replies.storage` with it. The fixture's theme, panel width, scale and, if it gives one, height are used; otherwise the panel's own height. The guest sees the fixture's clock, seed and time zone, so two renders of one fixture on one Mac are the same bytes. The render answers host requests from the fixture's `replies`, through the broker and with the fixture's `consent`, exactly as `test` does, so `search` and `items` show what they would in the launcher. Every build of Dupin can render, the released app included; only a build made with `--no-default-features` leaves Rune's headless renderer out, and `snap` then fails with `render-failed`.

### Fixtures

A fixture is one JSON file in `fixtures/`, read in name order. Symbolic links are refused, so a fixture can't make the runner read outside the package.

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

| Key           | Value                                                                                                                       |
| ------------- | --------------------------------------------------------------------------------------------------------------------------- |
| `fixture`     | Required. The format version, `1`. A fixture of another version is reported, not guessed at.                                |
| `description` | Optional text for people.                                                                                                   |
| `environment` | Optional. What the run would otherwise take from this Mac; see below.                                                       |
| `replies`     | Optional. What the host services answer; see below. Left out, every host request fails as `unavailable`.                    |
| `steps`       | Required. 1–64 steps, the first a `query`. Each has exactly one of `query`, `select`, `activate` and `event`, and `expect`. |

| `environment` key | Default                        | Fixes                                                                                                      |
| ----------------- | ------------------------------ | ---------------------------------------------------------------------------------------------------------- |
| `now`             | `"2026-01-01T09:00:00Z"`       | The clock, in UTC with optional milliseconds. `Date.now()` and `new Date()` return it and it doesn't move. |
| `seed`            | `1`                            | `Math.random`, which returns the same numbers for the same seed.                                           |
| `timeZone`        | `"UTC"`                        | The extension host's time zone, a tz database name this Mac has, for local-time `Date` methods.            |
| `locale`          | `"en-GB"`                      | `LANG` for the headless render. The guest has no `Intl`, so no answer depends on it yet.                   |
| `theme`           | `"dark"`                       | `snap`'s appearance, `dark` or `light`.                                                                    |
| `window`          | `{ "width": 680, "scale": 2 }` | `snap`'s panel width (400–1200 points), scale (1–3) and, optionally, `height` (60–2000 points).            |

| Step                                  | Does                                                                                                                                                                            | Its outcome                                                                       |
| ------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| `{ "query": "…" }`                    | Types the query after the keyword, replacing the last, and waits for the answer. The first row is selected.                                                                     | `rows`, `selected` when there are rows, and `error` when the search failed.       |
| `{ "select": 2 }`                     | Selects a row, counting from 0.                                                                                                                                                 | `selected`                                                                        |
| `{ "activate": "Copy" }`              | Chooses the selected row's action with this title or id. A `run` action calls the package's `actions` handler as Enter does; Copy and Open URL are recorded, never carried out. | `effect`, and for `run`, `toasts` or `error`.                                     |
| `{ "event": "clipboard", "data": … }` | Calls `events[event](data)`, as a watch Dupin keeps for the package would.                                                                                                      | `invalidated` when the handler invalidated the catalogue, `error` when it failed. |

Any step's outcome also has `clipboard`, the texts the package wrote to the clipboard during it, and `storage`, the whole store as each record's `data`, oldest first and without blobs, when the step changed it. `expect` names any of `rows`, `selected`, `effect`, `toasts`, `clipboard`, `storage`, `invalidated`, `error`, `sections` and `detail`; only the keys it names are compared, each as canonical JSON, and a key the outcome lacks compares as `null`. Nothing produces `sections` or `detail` until views do. A row is `{ "id", "title", "subtitle", "keywords", "icon", "section", "detail", "actions" }`, each optional field only when the row has it, where `detail` is `{ "markdown", "metadata" }` and each action is `{ "title", "copy" | "paste" | "url" | "run" }`; `id` is the item's own or `#0`, `#1` and so on. A step's `activate` may name an action by the ID the launcher gives it: `copy`, `paste` and `open` for a row's first Copy, Paste and Open URL, and `action-<index>` for the others. A query step answers as the keyword does: `search`'s rows for a query that isn't blank, then the `items()` catalogue's matches, ranked by Dupin's own matcher (without usage history, which a fixture doesn't have) and losing their `section` once the query ranks them. The fixture holds the catalogue from step to step as the launcher does, so an `activate` or `event` step whose handler calls `items.invalidate()` has `invalidated: true` in its outcome, and the next query asks `items` with `reason` `"invalidated"` and the cursor and applies the update it answers. An `error` is the diagnostic: `code`, `message`, `file`, `field`, `line`, `column` and `hint`, as `dupin ext run --json` reports it.

`replies` answers host requests in place of this Mac. Every request still goes through the broker, against the scopes `package.json` declares, so an undeclared origin, a method or path out of scope, or a clipboard read while typing fails exactly as in the launcher, before any reply is looked up. A request the fixture has no reply for fails rather than reaching the network, a program or a file of yours.

| `replies` key | Value                                                                                                                                                                                                                                                                      |
| ------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `http`        | `[{ "request": { "method", "url" }, "response": { "status", "headers", "body" or "json" } }]`. `method` defaults to `GET` and `status` to `200`; the URL must match exactly. Any other request fails with `network`.                                                       |
| `files`       | Each declared `files` scope's contents by name: a string for a `file` scope, or an object of relative paths and texts for a `directory` scope. Each scope is bound to a scratch folder holding exactly these files, empty when left out.                                   |
| `exec`        | `[{ "profile", "args", "status", "stdout", "stderr" }]`. `profile` is the declared name, `@script:` included; `args`, if given, must equal the arguments after the inputs fill the template. Nothing is run.                                                               |
| `storage`     | The `data` of each record the package's storage holds when the fixture starts, oldest first, stored within its declared quota in a scratch store that ends with the fixture.                                                                                               |
| `clipboard`   | What a clipboard read returns, such as `{ "type": "text", "text": "…" }`, or `null` for an empty clipboard. Watched copies come as `event` steps.                                                                                                                          |
| `dictionary`  | Each term's entries, in the [dictionary](#dictionary)'s shape. Any other term fails with `unavailable`.                                                                                                                                                                    |
| `calendar`    | The events a read can see, in the [calendar](#calendar)'s shape; a read returns those overlapping its range that match its `query`, in its `order`. Whole days are counted from `now` in UTC.                                                                              |
| `consent`     | Your decision on each declared scope, named as grant files name it: `"allow"`, `"deny"` (granted, then revoked) or `"pending"` (`needs-consent`). A scope left out is allowed, and macOS access counts as allowed. Naming a scope the package doesn't declare is an error. |

### Through MCP

[`dupin mcp --workspace DIR`](./mcp.md#extension-tools) offers `ext_check`, `ext_run`, `ext_test` and `ext_snap` to an agent, for packages inside `DIR`. They are the commands above, offline: no build runs, nothing is written to the workspace (`ext_test` never updates), no grant you made applies and nothing asks macOS for access. `ext_snap` returns the PNG itself.

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
| `dupin ext list`          | Lists installed packages with their source, commit, state (installed, missing or modified) and the scopes granted for the installed revision.                    |
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
```

`requested` lists the scopes `dupin.capabilities` declares (see [Asking for more](#asking-for-more)), named as the [capability broker](../capability-broker.md) names grants: an `http` entry for `https://api.example.com` requests `http:https://api.example.com`, a `files` entry named `notes` requests `files:notes`, and `"clipboard": ["write"]` requests `clipboard:write`. A declaration the broker rejects stops the install. The lock holds no grants: what you allow is in `grants.toml` beside it, which the broker owns. When Dupin loads an updated package, its grants of scopes the new revision declares no wider carry over, scopes it widens or adds wait for you to allow them, and scopes it drops are forgotten; `dupin ext update` lists what it adds and drops. `dupin ext remove` forgets the package's grants too, and so does installing a name that wasn't installed, so a package never inherits what you allowed another of that name. A running Dupin reads `grants.toml` again before it next decides or saves, so it neither keeps nor writes back what the command forgot. A lock written by an earlier Dupin may still have `granted` and `pending_consent` lists, which nothing ever filled; they are ignored.

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
      grants = [
        "http:https://api.example.com"
        "files:notes"
      ];
      bindings."files:notes" = "/Users/me/Notes";
      deny = [ "clipboard:read" ];
    }
  ];
};
```

Each package links read-only into the extensions folder under `name`, which defaults to `name` in its `package.json`; set `name` to avoid reading the package while Nix evaluates. Its grants go to `extension-grants.toml` beside `config.toml`, the broker's read-only base, in the broker's own format:

```toml
version = 1

[packages.word-count]
revision = "blake3:…"
enabled = true
grant = ["http:https://api.example.com", "files:notes"]
deny = ["clipboard:read"]
bindings = { "files:notes" = "/Users/me/Notes" }
```

| Option     | Meaning                                                                                                               |
| ---------- | --------------------------------------------------------------------------------------------------------------------- |
| `grants`   | Scopes allowed ahead of time, for exactly this `src`. A `files` or `exec` scope also needs its path in `bindings`.    |
| `bindings` | The absolute path a granted `files` or `exec` scope is bound to: the file or folder it reads, or the program it runs. |
| `deny`     | Scopes the package may never be granted, even from Dupin.                                                             |
| `enable`   | `true` by default. `false` keeps the package off whatever you choose in Dupin.                                        |

A grant applies only to the `revision` it names: `dupin ext revision DIR` prints it, and Home Manager runs it on `src` when the configuration builds, so changing `src` needs the new revision and nothing in an edited copy is granted. Choices made in Dupin, in `grants.toml`, can take a declared grant away but never widen it, and a denied scope can't be allowed. The base applies to each package by its `package.json` name, whatever `name` calls its folder, and two packages of one name fail the build. Dupin reads the base again as soon as it changes, so a denial, `enable = false` or a grant taken away applies at once, but a new grant waits until Dupin restarts. macOS still asks for its own permissions only when you choose Request Access. These packages are pinned by Nix, so they aren't in `extensions.lock`, and `dupin ext update` and `remove` leave them to Home Manager.

## Wasm extensions

Dupin no longer runs WebAssembly extensions. A folder that still holds an `extension.toml`, and no `package.json` with a `dupin` key, is skipped, and Settings → Extensions and `dupin ext check` say `extension.toml: Wasm extensions were removed; port to JavaScript`. A `package.json` that isn't valid JSON or passes 64 KiB reports its own error instead, so a port in progress points at the line to fix. To port one, write its `search` in JavaScript as [above](#a-javascript-extension), or start from `dupin ext new`; once `package.json` has a `dupin` key, a leftover `extension.toml` is ignored. Pushed views have no JavaScript equivalent yet.

## Cost

The JavaScript engine ([QuickJS-ng](https://quickjs-ng.github.io/quickjs/) through [`rquickjs`](https://github.com/DelSkayn/rquickjs)) is compiled into Dupin at `-O3`, which adds about 1 MiB to the signed app; the extension host is the same executable, so it adds no file. The extension host compiles a package to bytecode when Dupin loads the folder, so a query starts a fresh engine from bytecode and pays only for execution. On a Blacksmith M4 runner, a 400-candidate search returning 50 rows took 1.4 ms at the median and 1.8 ms at p95 over 2,000 queries to a running host, bytecode included. Compiling that package, which starts a host for the compile, took 11.7 ms at the median and 12.0 ms at p95; a first query from kept bytecode took 10.3 ms and 12.0 ms. Most of a start is the cost of starting Dupin's executable. The host's footprint after the warm queries was about 6.2 MiB. `cargo test --release -p dupin --test extension_host -- --ignored --nocapture` repeats the measurement.

A catalogue costs nothing per keystroke but matching. With the 5,000-item fixture in `scripts/latency/catalogue.extensions`, one run of `nu scripts/latency.nu run --only catalogue` on a Blacksmith M4 runner put keypress to results at 2.0–2.5 ms at the median and 3.1 ms at p95 under its keyword, of which matching the rows took 1.0–1.3 ms at p95, and 3.5 ms at p95 for the main search with the catalogue in it.
