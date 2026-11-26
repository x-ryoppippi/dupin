# Extension capabilities

An extension reaches nothing on its own. It declares the host services it wants under `capabilities` in the `dupin` key of its `package.json`, and you allow each scope in the panel or in Settings → Extensions.

```json
"capabilities": {
	"http": [{ "origin": "https://api.github.com", "methods": ["GET"] }],
	"storage": { "quotaBytes": 1048576 }
}
```

| Capability                         | Scope names                                    | While you type                           | Needs an action you chose | macOS access  |
| ---------------------------------- | ---------------------------------------------- | ---------------------------------------- | ------------------------- | ------------- |
| [`http`](#http)                    | `http:<origin>`                                | GET and HEAD                             | Any other method          |               |
| [`files`](#files)                  | `files:<name>`                                 | Reading, listing                         |                           |               |
| [`exec`](#exec)                    | `exec:<name>`, `script:<name>`                 | A `catalogue` profile, for the catalogue | Running any other program |               |
| [`storage`](#storage)              | `storage`, `storage:sync`                      | Everything                               |                           |               |
| [`dictionary`](#dictionary)        | `dictionary`                                   | Lookups                                  |                           |               |
| [`calculator`](#calculator)        | `calculator`                                   | Evaluating                               |                           |               |
| [`snippets`](#snippets)            | `snippets`                                     |                                          | Changing snippets         |               |
| [`calendar`](#calendar)            | `calendar`                                     | Reading events                           |                           | Calendars     |
| [`windows`](#windows)              | `windows:read`, `windows:arrange`              | Reading                                  | Moving a window           | Accessibility |
| [`context`](#context)              | `context:frontmost`, `context:document`        | Reading                                  |                           |               |
| [`clipboard`](#clipboard)          | `clipboard:read`, `:write`, `:watch`, `:paste` | Watching                                 | Reading, writing, pasting | Yes           |
| [`rows`, `query`](#rows-and-query) | `rows:<kind>`, `query`                         | Filters                                  | Kind actions              |               |

<details>
<summary>Every capability declared</summary>

```json
"capabilities": {
	"http": [{ "origin": "https://api.github.com", "methods": ["GET"] }],
	"files": { "notes": { "kind": "directory", "access": ["read", "list"] } },
	"exec": { "git-status": { "args": ["status", "--short", "{path}"], "output": true }, "@script:open-in-zed": {} },
	"storage": { "quotaBytes": 1048576, "retentionDays": 30 },
	"clipboard": ["read", "write", "watch", "paste"],
	"dictionary": ["lookup"],
	"calculator": ["evaluate"],
	"snippets": ["write"],
	"calendar": { "access": ["read"], "pastDays": 0, "futureDays": 30 },
	"windows": ["read", "arrange"],
	"context": ["frontmost", "document"],
	"rows": { "app": ["title"], "file": ["path"] },
	"query": ["read"]
}
```

Watching the clipboard runs in the background; `windows:read` covers the displays and the focused window; `storage` reads, writes and deletes all work while you type. Reading and watching the clipboard need macOS to let Dupin paste from other apps; pasting needs Accessibility.

</details>

## Asking for more

Dupin checks every request against the declaration, the version you allowed and your choices.

- A declaration is the most an extension can get; each item in it is a scope you allow on its own.
- An extension that declares capabilities starts off, and each scope asks once: **Allow _Name_ to …?**, with **Allow** and **Deny**.
- An update that widens a scope asks again.

[Permissions](./permissions.md#extensions) and [Capability broker](../capability-broker.md) have the full rules.

<details>
<summary>The rules in full</summary>

- **A declaration is the most an extension can get.** Anything malformed, unknown or out of range rejects the package with a diagnostic naming the field, such as `dupin.capabilities.http[0].origin`.
- **Each item is a scope you allow on its own**: one origin, one named folder, one profile. Its name, such as `http:https://api.github.com`, `files:notes`, `exec:git-status`, `script:open-in-zed`, `storage`, `clipboard:read`, `dictionary`, `calculator`, `snippets`, `calendar`, `windows:arrange`, `context:document`, `rows:file` or `query`, is what errors, Settings and grant files call it.
- **It starts off.** An extension that declares capabilities starts off. Its first request while off fails with `disabled`, and the panel offers **Turn on _Name_?**. Turning it on grants nothing.
- **Each scope asks once.** A request for a scope you haven't decided on fails with `needs-consent`, and the panel shows **Allow _Name_ to …?** above the extension's rows, with **Allow** and **Deny**, until you answer.
- **Choices belong to a version.** A change to `package.json` or any module is another revision: one that declares a scope no wider keeps your choice, one that widens it asks again, and one that drops it forgets it.

</details>

<details>
<summary>Allowing, denying and revoking</summary>

Settings → Extensions lists the same requests as the panel, and every scope each extension declares with whether it is allowed, with **Allow**, **Revoke** and **Request Access**. A files or exec scope is allowed from Settings, where you pick the file, folder or program; a profile that names its program needs no pick. Allowing runs nothing the extension asked for before; use it again. **Deny** revokes the scope until you allow it in Settings.

Revoking a scope or turning an extension off takes effect at once: a request in flight finishes, but its result is dropped, the query's rows are dropped with it, and allowing again doesn't bring them back.

</details>

<details>
<summary>What needs an action you chose</summary>

While you type, an extension may use what you allowed to read: GET and HEAD, files, the dictionary, the calculator, calendars, the displays and the focused window, the app and file you were using, and its own `storage`, writes included. Any other HTTP method, a program or script, reading, writing or pasting from the clipboard, moving a window and changing your snippets need an action you chose. Dupin, not the extension, says which: a query, an event, Dupin asking for the catalogue or an action of yours. The one exception is an exec profile declared `catalogue`, which also runs when Dupin asks for the catalogue: on its first use, and in the background once its `ttl` passes, never for a keystroke.

</details>

<details>
<summary>macOS access</summary>

Calendars, reading or watching the clipboard, pasting and windows also need macOS access. Dupin never asks macOS on its own: not at startup, on opening Settings, on reload or while an extension runs. Turning on a bundled extension that ships off, such as Window Management, is turning on its feature and asks for what its grants need; turning on any other extension never asks. A request that needs it fails with `needs-consent` and `permission` set, and **Request Access** in the panel or Settings asks macOS once.

</details>

## Errors

A request that fails rejects with a `DupinError` whose `code` the extension can branch on, such as `needs-consent`, `needs-activation` or `out-of-scope`.

```js
if (error instanceof DupinError && error.code === 'needs-consent')
	return { type: 'toast', text: `Allow ${error.description} first` };
```

<details>
<summary>Every code, and an example</summary>

`scope`, `description` and `permission` say what consent is missing.

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

| `code`             | Meaning                                                                                                                                |
| ------------------ | -------------------------------------------------------------------------------------------------------------------------------------- |
| `needs-consent`    | You haven't allowed the scope, an update widened it, or macOS hasn't been asked. Dupin shows the request.                              |
| `needs-activation` | Only an action you chose may do this.                                                                                                  |
| `disabled`         | The extension is off.                                                                                                                  |
| `revoked`          | You revoked the scope, or its access changed while the request ran and the result was dropped.                                         |
| `denied`           | Your declared configuration denies the scope.                                                                                          |
| `os-denied`        | macOS denied or restricts the access; it changes in System Settings → Privacy & Security.                                              |
| `undeclared`       | The package doesn't declare the scope, such as an origin it didn't list.                                                               |
| `out-of-scope`     | The request goes beyond the declaration: another method, a files access, a wider calendar window, or a window frame off every display. |
| `quota-exceeded`   | A storage write would pass the declared quota.                                                                                         |
| `invalid`          | An argument is malformed or out of range.                                                                                              |
| `too-many-calls`   | More than 16 requests wait at once, or one call made more than 256.                                                                    |
| `too-large`        | A request or its result is larger than Dupin passes (2 MiB out, 4 MiB back).                                                           |
| `unavailable`      | No service answers here, as in `dupin ext run`, or this build has none for the capability yet.                                         |
| a service's code   | Such as `not-found`, `timeout`, `blocked-address` or `failed`, with the service's message.                                             |

A request counts against its call's deadline: 2 seconds while you type, 30 seconds for an action.

</details>

## HTTP

`"http": [{ "origin": "https://api.github.com", "methods": ["GET"] }]` declares up to 32 HTTPS origins, each with its methods; `http.fetch(url, { method, headers, body })` calls them.

```js
const response = await http.fetch(
	`https://api.github.com/search/repositories?q=${encodeURIComponent(query)}`,
);
```

<details>
<summary>Responses, limits and an example</summary>

`http.fetch` returns a response with `ok`, `status`, `url`, `redirected`, `headers` (lowercase names, without cookies), `truncated`, `text()` and `json()`:

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

| Limit           | Value                   |
| --------------- | ----------------------- |
| Request headers | 64                      |
| Request body    | 1 MiB                   |
| Response        | 2 MiB, then `truncated` |

Dupin sends no cookies or credentials of its own, connects only to the permit's origin, follows a redirect only within it, and refuses private and reserved addresses.

</details>

## Files

`"files": { "notes": { "kind": "directory", "access": ["read", "list"] } }` declares named scopes; you pick the file or folder each one is when you allow it in Settings.

| Function                      | Returns                                             |
| ----------------------------- | --------------------------------------------------- |
| `files.list(scope, path?)`    | Entries with `name`, `kind`, `size` and a `handle`  |
| `files.readText(scope, path)` | The text of a file                                  |
| `files.read(scope, path)`     | The text, its `size` and whether it was `truncated` |

<details>
<summary>Names, paths and an example</summary>

A files scope's name is lowercase letters, digits, `-` and `_`. Paths are relative to the scope with `/`, and the [files capability](../extension-capabilities.md#files) describes how they are checked.

```js
import { files } from 'dupin';

export async function search(query) {
	const notes = await files.list('notes');
	return notes
		.filter((entry) => entry.kind === 'file' && entry.name.includes(query))
		.map((entry) => ({ title: entry.name }));
}
```

</details>

## Exec

`"exec"` declares named profiles, each an argument template, or `@script:<name>` for one of your [script commands](./scripts.md). `exec.run(profile, inputs)` runs one, only in an action; you pick the program when you allow it, unless the profile names one of macOS's own.

<details>
<summary>Profiles, results and an example</summary>

Each profile's `{input}` placeholders take one line of text. A script needs no pick. `output: true` asks to receive stdout and stderr, each up to `outputBytes` (64 KiB unless declared, 1 MiB at most). `exec.run` returns `code`, `signal`, `timedOut` and, with output allowed, `stdout`, `stderr` and `truncated`.

| Profile field | Value                                                                                                                                                                                                                                                                                                       |
| ------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `args`        | Required. The argument template, up to 32 entries.                                                                                                                                                                                                                                                          |
| `output`      | `true` hands the extension stdout and stderr.                                                                                                                                                                                                                                                               |
| `outputBytes` | With `output`, the most of each stream it receives: 1 byte to 1 MiB, 64 KiB without it.                                                                                                                                                                                                                     |
| `program`     | A program of macOS's own, in `/bin`, `/sbin`, `/usr/bin`, `/usr/sbin`, `/usr/libexec` or `/System` outside `/System/Volumes`, which System Integrity Protection keeps from being replaced. Consent names it, and you pick nothing. A bundled extension that ships with it holds the profile from the start. |
| `catalogue`   | `true` lets the profile run when Dupin asks for the extension's [catalogue](./extension-api.md#itemsctx), without an action of yours, so a list of what a program reports refreshes on its own. Its `search`, views and event handlers still can't run it, and an action still can.                         |

A catalogue profile is how [Processes](./processes.md), [Ports](./ports.md) and [Coding Agents](./coding-agents.md) list what a program such as `ps` reports every few seconds while you look at it, without running a program as you type: Dupin asks for the catalogue on first use and then in the background each time its `ttl` passes.

```json
"exec": {
	"processes": { "program": "/bin/ps", "args": ["-axww", "-o", "pid=,comm="], "output": true, "outputBytes": 1048576, "catalogue": true },
	"terminate": { "program": "/bin/kill", "args": ["-TERM", "{pid}"] }
}
```

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

Programs and scripts run with your own access, without a shell, as the [exec capability](../extension-capabilities.md#exec) describes; consent shows the program and every argument, each in quotes, in full. An argument may not hold control or invisible formatting characters, which could hide or reorder what consent shows.

</details>

## Storage

`"storage": { "quotaBytes": 1048576, "retentionDays": 30 }` gives the package an encrypted store of its own: records of JSON, each with the blobs (byte strings, such as an image) it carries.

```js
const record = await storage.put({ title: 'Cover' }, { blobs: [thumbnail] });
const { records, next } = await storage.list({ limit: 20 });
```

> [!NOTE]
> Revoking `storage` keeps the data, so allowing it again finds it; `dupin ext remove` deletes it.

With `"sync": true` the package also asks for `storage:sync`, which copies what it stores, encrypted, to your [sync](./sync.md) folder and your other Macs.

<details>
<summary>Syncing it</summary>

A package that declares `"storage": { "quotaBytes": …, "sync": true }` asks for `storage:sync` too, shown as "copy what it stores, encrypted, to your sync folder and your other Macs". Granted on a Mac while [sync](./sync.md) is on, its namespace is sealed under the vault key and syncs, the later write of a key winning, under a name taken from the package's Git source and name, so the same package on two Macs shares its records.

It is granted on each Mac separately; a package copied from a folder, declared with Nix or bundled has no name another Mac shares and never syncs its storage. Revoking it, or turning sync off, keeps what the package stored here and stops syncing at once; its files leave the folder once the other Macs that sync it have taken them over.

</details>

<details>
<summary>Functions and declaration</summary>

It works while you type, in actions and in event handlers, once you allow it.

| Function                    | Does                                                                                                                                            |
| --------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `put(data, { blobs, key })` | Stores a record and returns it: `{ id, createdAt, data, blobs: [{ handle, size }] }`. With `key`, it replaces the record stored under that key. |
| `get(id)`                   | The record, or `undefined`.                                                                                                                     |
| `list({ before, limit })`   | Up to `limit` records (50 by default, 200 at most) older than `before`, newest first, and `next` to pass as `before` for the following page.    |
| `readBlob(handle)`          | The blob's bytes as a `Uint8Array`, while a record still carries it.                                                                            |
| `delete(id)`, `clear()`     | Delete one record, or all of them. A blob goes with the last record that carries it.                                                            |
| `usage()`                   | `{ entries, bytes, quotaBytes, retentionDays }`.                                                                                                |

| Declaration     | Range        | Meaning                                                                  |
| --------------- | ------------ | ------------------------------------------------------------------------ |
| `quotaBytes`    | Up to 64 MiB | The most the package keeps in all, beside at most 10,000 records.        |
| `retentionDays` | 1–3,650      | Records older than this are deleted; without it they stay until deleted. |
| `sync`          | `true`       | Also asks for `storage:sync`; see Syncing it above.                      |

```js
import { storage } from 'dupin';

const thumbnail = new Uint8Array([0x89, 0x50, 0x4e, 0x47]);
const record = await storage.put({ title: 'Cover' }, { blobs: [thumbnail] });
const { records, next } = await storage.list({ limit: 20 });
const bytes = await storage.readBlob(record.blobs[0].handle);
```

</details>

<details>
<summary>Records and blobs</summary>

- Blobs are typed arrays, `ArrayBuffer`s or the `blob` token of the latest [clipboard change](./extension-api.md#events).
- A `key` is up to 256 bytes. The record stored under it is replaced, keeps its ID and lists as the newest.
- A `list` page also ends before 2 MiB of JSON.
- Record IDs and blob handles are opaque strings. IDs are random, so they don't sort by when the record was stored; `list` gives that order.
- The same blob stored twice is kept once and counts once.
- Writes, reads and deletes all need the grant; none needs an action you chose.

</details>

<details>
<summary>Limits</summary>

A record's JSON and the blobs the extension sends are at most 1 MiB together, with at most 16 blobs; a larger one fails with `too-large`. A clipboard image stored by its token may add up to 32 MiB, but `readBlob` hands the extension at most 4 MiB. The package keeps up to 10,000 records and `quotaBytes` in all. A `put` that doesn't fit fails with `quota-exceeded` and deletes nothing; Dupin checks the room and writes in one step, so writes made at the same time can't pass the quota together. An update that lowers the quota or the retention deletes the oldest records that no longer fit.

</details>

<details>
<summary>Where it lives and when it is deleted</summary>

Each package has a [`dupin-storage`](./storage.md) namespace of its own, named by a hash of its package ID and where it was installed from, so no package reads another's data, a native feature's, or that of a bundled package it copies. Everything is sealed with the data key in your Keychain.

Revoking `storage` in Settings or the panel stops every request at once and keeps what the package stored, so allowing it again finds it; Settings → Extensions offers **Delete Data** beside the scope. `dupin ext remove` deletes the package's data with it, asking the running Dupin to if it has the data open. The namespace follows the package name, so `dupin ext install` of a name that isn't installed also deletes what an earlier package of that name left behind, and doesn't install while that data can't be deleted.

</details>

## Dictionary

`"dictionary": ["lookup"]` lets `dictionary.lookup(term)` look a term up while you type, in the dictionaries turned on in [Dictionary](./dictionary.md)'s settings. It needs no macOS permission.

<details>
<summary>What a lookup returns</summary>

`dictionary.lookup(term)` takes a term of at most 200 characters and returns up to 8 entries, each as the dictionary wrote it (`text`), laid out as the bundled Dictionary shows it (`markdown`, ready for an item's `detail`), and as structure:

```json
{
	"headword": "run",
	"dictionary": null,
	"pronunciation": "rʌn",
	"text": "run | rʌn | verb (runs, running; past ran; past participle run) 1 [no object] move on foot faster than walking: she ran to the station. • (of a machine) operate: the engine runs on diesel. ORIGIN Old English rinnan.",
	"markdown": "### Verb\n\n\\(runs\\, running\\; past ran\\; past participle run\\)\n\n1. \\[no object\\] move on foot faster than walking: *she ran to the station\\.*\n   - \\(of a machine\\) operate: *the engine runs on diesel\\.*\n\n## Origin\n\nOld English rinnan\\.",
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

Text the layout doesn't recognize stays in a sense's `definition` or a group's `note`, and the Markdown as paragraphs, so nothing is lost. An entry keeps up to 16 KiB of `text`, 32 KiB of `markdown`, 32 groups, 64 senses each with 16 examples and one level of sub-senses, and 16 sections, each structured text cut to 4 KiB; `truncated` says whether anything was cut. macOS doesn't say which dictionary an entry came from, so `dictionary` is `null`.

</details>

## Calculator

`"calculator": ["evaluate"]` lets an extension work things out while you type with the calculator the bundled [Calculator](./calculator.md) uses. It needs no macOS permission and reaches nothing outside Dupin.

```js
const answer = await calculator.evaluate('5 km to mi'); // { result: 'approx. 3.1068559612 mi', … }
```

<details>
<summary>Answers and what it refuses</summary>

```js
const answer = await calculator.evaluate('5 km to mi');
// { result: 'approx. 3.1068559612 mi', number: '3.1068559612',
//   parts: [{ kind: 'ident', text: 'approx. ' }, { kind: 'number', text: '3.1068559612' }, { kind: 'ident', text: ' mi' }] }
```

`calculator.evaluate(expression)` takes up to 512 bytes on one line and answers within 50 ms, or `null` for anything it doesn't answer. `parts` is the answer in order: numbers, `ident`s (units and currencies, with the space beside them) and anything else.

An empty expression, a longer one or one with a line break or another control character fails with `invalid` before anything is worked out. It answers `null` for brackets that don't balance or nest more than 17 deep, characters other than letters, digits, spaces, `+ - − * × / ÷ ∕ ^ % ! . , _ °`, brackets and `$ € £ ¥ ₺`, an error, or an answer longer than 256 bytes.

</details>

<details>
<summary>Currencies</summary>

While **Currency conversion** is on in Settings → Search, currencies are units too, at the European Central Bank's reference rates, which Dupin downloads itself, and an answer that used one has `exchange`:

| `exchange`                                  | Means                                                                                                 |
| ------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| `{ status: 'ok', date, stale, currencies }` | The rates' date, whether Dupin couldn't refresh them in the past 12 hours, and the currencies it has. |
| `{ status: 'unavailable' }`                 | Dupin has no rates yet.                                                                               |
| `{ status: 'missing', currency }`           | The bank doesn't quote that currency.                                                                 |

With either of the last two, `result` was worked out with a placeholder rate and isn't to be shown.

</details>

## Snippets

`"snippets": ["write"]` lets `snippets.save` and `snippets.delete` change the [snippets](./snippets.md) you store in Dupin, only from an action you chose or a form you submitted. It can't read them or touch `config.toml`'s.

<details>
<summary>Functions, checks and consent</summary>

A search, view or event handler that tries fails with `needs-activation`.

```js
export const actions = {
	async save(values, ctx) {
		const problems = await snippets.save(values, { replaces: ctx.state?.keyword ?? null });
		if (Object.keys(problems).length > 0) return { type: 'fieldErrors', errors: { ...problems } };
		return { type: 'pop', toast: 'Saved' };
	},
};
```

| Function                                               | Does                                                                                                                                            |
| ------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `snippets.save({ keyword, name, text }, { replaces })` | Stores a snippet in place of the stored one keyed `replaces`, or as a new one without it; resolves to what is wrong by field, `{}` once stored. |
| `snippets.delete(keyword)`                             | Deletes a stored snippet and resolves to whether there was one.                                                                                 |

The package Dupin ships for Snippets is handed the snippets as its preferences, and no other package is. The shipped package holds the scope from the start; any other package, an edited copy of it included, asks, as the consent line **add, change and delete the snippets you store in Dupin** says.

Dupin checks a snippet exactly as [Snippets](./snippets.md#configuration) describes: a keyword no other snippet has, in storage or `config.toml`, a name or none, the text, and Dupin's limits. A `replaces` that names no stored snippet is refused on `keyword`, so a package can't edit a `config.toml` snippet as a stored one. Before any check, a keyword, name or `replaces` over 1 KiB, text over 128 KiB, or a field of the wrong type fails with `invalid`.

</details>

## Calendar

`"calendar": { "access": ["read"], "pastDays": 1, "futureDays": 30 }` lets `calendar.events({ from, to, query, limit, order })` read events within the declared whole days before and after today. It also needs macOS Calendars access, which only **Request Access** asks for.

```js
const events = await calendar.events({ from: now, to: tomorrow });
```

<details>
<summary>Options and an example</summary>

The bundled [Calendar search](./calendar.md) is written this way.

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

| `calendar.events` option | Value                                                                                                                                             |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `from`, `to`             | A `Date`, an ISO 8601 string or JavaScript milliseconds; `to` is exclusive. Wider than the declared days is `out-of-scope`, not quietly narrowed. |
| `query`                  | Up to 200 characters: only events whose title or location holds every word of it, in any order and case.                                          |
| `limit`                  | Up to 500 events, 100 by default.                                                                                                                 |
| `order`                  | Earliest start first, the limit keeping the earliest; `order: 'latest'` for latest first, keeping the latest, as a read of past events wants.     |

With `query`, a search sends the extension only what matches.

</details>

<details>
<summary>An event</summary>

`start` and `end` are ISO 8601 in UTC:

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

- `id` names one occurrence of a recurring event.
- `color` is the calendar's colour in sRGB.
- A title comes as one line of up to 512 bytes, so it fits a row's `title` as it is; locations and names are cut to 1 KiB.
- `url` is the event's own link, if its calendar holds one.
- `link` shows the occurrence in Calendar: give it as an [`open` action](./extension-api.md#actions) of the `search` that read it, the only scheme besides `http` and `https` one opens. A link no read in that search returned, or one in an `items()` catalogue, fails the answer with `invalid-url`.
- The guest has no `Intl`, so `labels` has the event's dates as this Mac's locale and time zone write them: `day` the weekday and date it starts, `time` only its time, or `All day`, when it falls within one day and `null` otherwise, and `dates` its dates and times, each with the year when it isn't this year.

</details>

## Windows

`"windows": ["read", "arrange"]` lets an extension read the displays and the focused window (`windows.displays()`, `windows.focused()`) and move it (`windows.setFrame(frame)`). Both need macOS Accessibility access. The bundled [Window Management](./window-management.md) is written this way.

<details>
<summary>Functions, frames and an example</summary>

Each kind of access is declared separately. Both act on the focused window of the app that was in front when the panel opened, and Accessibility access is asked for only by **Request Access**, or by turning on a bundled extension that ships off.

```ts
import { windows } from 'dupin';
import type { Actions } from 'dupin';

export const actions: Actions = {
	async leftHalf() {
		const [display] = await windows.displays();
		const { x, y, width, height } = display.visibleFrame;
		await windows.setFrame({ x, y, width: Math.round(width / 2), height });
	},
};
```

| Access    | Function                                    | Does                                                                                                                                                         |
| --------- | ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `read`    | `windows.displays()`                        | Every display, the primary one first, as `{ frame, visibleFrame, scale }`; `visibleFrame` leaves out the menu bar and the Dock, `scale` is pixels per point. |
| `read`    | `windows.focused()`                         | `{ frame, app: { name, bundleId } }`, or `null` when that app has no window or Dupin itself was in front; the displays are read either way.                  |
| `arrange` | `windows.setFrame({ x, y, width, height })` | Moves and resizes that window when you choose an action and resolves to the frame it took, which the app may constrain.                                      |

Frames are in points, in one space for every display: the origin is the primary display's top-left corner and `y` grows downwards. The extension computes where a window goes; Dupin only reads and moves it.

A frame that lies off every display is `out-of-scope`, and one that isn't finite, starts more than 1,000,000 points from the origin or is not from above 0 to 100,000 points wide and tall is `invalid`.

</details>

## Context

`"context": ["frontmost", "document"]` lets an extension read where you were when the panel opened: `context.frontmost()` is the app that was in front, and `context.document()` the file its focused window showed. Each is a scope of its own, so a package can know you were in your editor without seeing which file you had open.

```js
const app = await context.frontmost(); // { name: "Zed", bundleId: "dev.zed.Zed", pid: 501 } or null
const document = await context.document(); // { path: "/Users/me/dupin/README.md" } or null
```

<details>
<summary>What each returns, and when it is null</summary>

| Access      | Function              | Returns                                                                                                                         |
| ----------- | --------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `frontmost` | `context.frontmost()` | `{ name, bundleId, pid }` of the app in front when the panel opened, or `null` when Dupin itself was; `bundleId` may be `null`. |
| `document`  | `context.document()`  | `{ path }`, the absolute path of the local file that app's focused window showed, or `null`.                                    |

Dupin notes the app as the panel opens, and reads the document the first time a package asks after that, once, so every answer holds until the panel opens again and typing never waits on the other app. Neither needs an action you chose, and neither asks macOS for anything:

- The document is what the app itself reports through Accessibility, as editors such as TextEdit, Xcode and Preview do. It is `null` for an app that reports none, such as a terminal or a browser, for anything that isn't a local file, and while Dupin isn't allowed Accessibility; Dupin uses that access only if another feature, such as Window Management or Selected text, already has it, and never asks for it for this.
- Text you had selected reaches a package as the query when you open Dupin with the [Selection hotkey](./selected-text.md), so there is no capability for it.

A package can put first what belongs to where you were, such as the project holding the open file:

```js
import { context } from 'dupin';

export async function search() {
	const document = await context.document().catch(() => null);
	return document ? [{ title: document.path.split('/').at(-1), subtitle: document.path }] : [];
}
```

</details>

## Clipboard

`"clipboard": ["read", "write", "watch", "paste"]` declares each kind of access separately.

| Access  | Allows                                                                           |
| ------- | -------------------------------------------------------------------------------- |
| `read`  | `clipboard.read()` and `clipboard.readText()`, in an action you chose.           |
| `write` | `clipboard.writeText(text)` and `clipboard.write(copy)`, in an action you chose. |
| `paste` | A row's [`paste` action](./extension-api.md#actions).                            |
| `watch` | [`events.clipboard`](./extension-api.md#events), for each new copy.              |

<details>
<summary>What each access does</summary>

| Access  | What it allows                                                                                                                                                                                             |
| ------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `read`  | `clipboard.read()` returns the current copy when you choose an action, or `null` when the clipboard is empty or the copy is one Dupin never reads; `clipboard.readText()` returns its text, if it is text. |
| `write` | `clipboard.writeText(text)` puts up to 1 MiB of text on the clipboard when you choose an action. `clipboard.write(copy)` also takes a stored PNG or files a watch handed over.                             |
| `paste` | A row's `paste` action puts its text in the app in front of Dupin when you choose it.                                                                                                                      |
| `watch` | `events.clipboard` receives each new copy, without your choosing anything.                                                                                                                                 |

Reading and watching need macOS to let Dupin paste from other apps without asking (**Always Allow**); pasting needs Accessibility access; writing needs no macOS permission.

- `clipboard.write(copy)` takes `{ type: "image", blob }`, a PNG the package stored, by its blob's handle, or `{ type: "files", files: [{ path, handle }] }`, files a watch handed the package with those handles that are still there; it fails with `out-of-scope` for any other file.
- For a paste, Dupin presses ⌘V, which needs macOS Accessibility access; until both are allowed, choosing it copies the text, shows why, and leaves the request in the panel and Settings. The extension's code never pastes: the action is data, and `clipboard:paste` only lets Dupin carry it out.
- Dupin's own clipboard monitor delivers watched changes, so the extension never polls, and reads each copy once however many watches there are; at most 16 run at once. Keeping a history is up to the extension, in its own `storage`.

</details>

<details>
<summary>What a copy holds and which copies are skipped</summary>

A copy is read in the one form clipboard history keeps, in this order of preference:

1. `{ "type": "files", "files": [{ "path", "name", "folder", "size", "kind" }] }`
2. `{ "type": "url", "url" }`
3. `{ "type": "text", "text" }`
4. an image converted to PNG and scaled to 4096 pixels on its longest side: `{ "type": "image", "mediaType": "image/png", "width", "height", "data": "<base64>" }`, up to 2 MiB for a read and `[clipboard] max_image_bytes` for a watch.

Files are described, never read, and Finder's names for them are never read as text. Like clipboard history, reads and watches skip copies that password managers mark concealed, transient or auto-generated, copies from the apps clipboard history ignores (`[clipboard] ignored_apps`, followed from the next copy when it changes), and text longer than its `max_entry_bytes`; Dupin checks those markers and apps before it reads the contents. Which app made a copy is only known if Dupin's clipboard monitor saw it land, by the app frontmost then, so while `ignored_apps` lists any app, a read returns `null` for a copy made before the clipboard service started or within the half second before the read.

</details>

## Rows and query

`rows` and `query` are what the package's [filters and kind actions](./advanced.md#extensions) may see of rows Dupin found (`rows:app`, `rows:file` and so on, with the fields declared) and of what you type. `query` is also what the bundled Calculator sees of the main search.

## Security model

Extension code runs in a sandboxed helper process that can't open files, use the network, connect to other processes or start programs; everything it reaches goes through the capabilities above. Install only extensions you trust.

<details>
<summary>How the extension host is confined</summary>

Dupin never parses, compiles or runs extension code itself; erasing TypeScript's types is part of the host's compile. The extension host is Dupin's own executable run with `--extension-host`. The bundled extensions share one host, started on the first query, and every other extension gets a host of its own, so no third-party extension's code runs where another extension's queries do; each package is compiled in a host started for that compile alone.

Before it touches any extension code, the host confines itself with a deny-by-default macOS sandbox: it cannot open or list files, use the network, connect to other processes or system services, or start programs. Its resource limits allow no new files, child processes or core dumps. Dupin reads each package's files within the [limits](./extension-api.md#limits) and sends their text over a private socket; that host compiles them under the same deadline, limits and watchdog as a query, counted from when it is ready, returns bytecode or a diagnostic and exits. A compile never waits for another extension's query, and a host one package has subverted never compiles another. Dupin keeps the bytecode as opaque bytes, reuses it while the files' content is unchanged and sends it back with every query, so the host never reads the extensions folder and a package can never supply bytecode of its own. Dupin checks every answer again before showing it. The host keeps the time zone it starts with, so `Date` reports local time.

The sandbox contains an extension that breaks out of the engine through a flaw in QuickJS-ng. A host answers each query's requests with the permissions of the extension the query is for, which is why an extension from the extensions folder never shares one: such an extension can reach only its own permissions. A host that sends anything Dupin didn't ask for is stopped.

</details>

<details>
<summary>Watchdog</summary>

The engine stops a query that runs past its deadline or is cancelled, but a few built-in operations, such as `indexOf` on very long strings, can't be interrupted, and the engine's heap limit doesn't bound the whole process. Dupin therefore watches the host from outside. It stops a host when a compile or query runs 150 ms past its deadline, ignores a cancellation for 150 ms or grows past its memory allowance, and checks every second that a host waiting for a query neither works nor grows, since only code nobody sent it could. When the host dies, the query becomes a "_Name_ failed" row. The next query starts a new host. Each host runs one query at a time; the others wait in a bounded queue, and a query that typing replaces leaves it. The host exits when Dupin quits and after two minutes without queries, and bytecode Dupin kept is sent to the next host, so a restart doesn't recompile anything.

</details>

<details>
<summary>Other platforms</summary>

The sandbox exists on macOS only. On other platforms the host refuses to compile or run extensions, and Settings lists each JavaScript extension as failing with `sandbox-unavailable`; Linux needs its own confinement first. The host refuses for the same reason when Dupin itself already runs inside another sandbox.

</details>
