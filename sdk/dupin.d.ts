// Types for Dupin extensions, apiVersion 2. Dupin never reads this file; it is for editors and
// `tsc --noEmit`. `dupin ext new` copies it into each package.
//
// Dupin runs a TypeScript `dupin.main` as written by erasing its types, without type-checking.
// Import these types with `import type { Item } from 'dupin'`: a type imported as a value stays
// in the module and fails to link. `enum`, namespaces with values, parameter properties,
// decorators and JSX need a build (`dupin ext build`), as do npm dependencies.
// Docs: https://github.com/x-ryoppippi/dupin/blob/main/docs/features/extensions.md

declare module 'dupin' {
	/** The guest API this file describes. The host's `dupin` module exports the same value. */
	export const apiVersion: 2;

	type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

	/** What `search` receives after the query. Dupin freezes it. */
	export interface SearchContext {
		/** How many rows Dupin will show, at most 50. Rows beyond it are dropped. */
		readonly limit: number;
	}

	/**
	 * One row. Enter runs its first action: Open URL when `url` is set, then Copy when `text` is
	 * set, then `actions` in order. Any other field fails the whole answer.
	 */
	export interface Item {
		/** One line, up to 512 bytes. */
		title: string;
		/** Up to 2,048 bytes. */
		subtitle?: string;
		/** Unique among the items, up to 128 bytes. Dupin numbers `search` items without one. */
		id?: string;
		/** An `http` or `https` URL with a host, up to 2,048 bytes. */
		url?: string;
		/** Non-empty, up to 16 KiB. */
		text?: string;
		/** An image in the package (`.png`, `.jpg`, `.jpeg`, `.svg`, `.pdf` or `.icns`), relative to `package.json`. */
		icon?: string;
		/** The heading the item is listed under; a section's items get its title. Up to 128 bytes. */
		section?: string;
		/** Shown while the item is selected. */
		detail?: DetailPane;
		/** Up to eight, counting the ones `url` and `text` add. */
		actions?: Action[];
	}

	/** Runs `actions[run](data, ctx)` when the user chooses it. */
	export interface ItemAction {
		/** One line, up to 512 bytes. */
		title: string;
		/** A key of the module's `actions` export: up to 64 letters, digits, `_` or `$`. */
		run: string;
		/** Passed to the handler as JSON, or `null` without it: at most 4 KiB, nested at most 16 deep. */
		data?: Json;
	}

	/** An item of an `items` catalogue, which may carry words that also find it. */
	export interface CatalogueItem extends Item {
		/**
		 * Required, and kept with the item as the list changes: Dupin applies updates and
		 * remembers which items are picked by it.
		 */
		id: string;
		/** Up to 16 words of up to 64 bytes, matched below an equal match on the title. */
		keywords?: string[];
	}

	export interface Metadata {
		/** One line, up to 64 bytes. */
		label: string;
		/** One line, up to 512 bytes. */
		text: string;
	}

	export interface DetailPane {
		/** Up to 32 KiB: headings, paragraphs, emphasis, lists, inline and fenced code and HTTP(S) links. */
		markdown?: string;
		/** Up to 12 entries, above the Markdown. */
		metadata?: Metadata[];
	}

	export type Action =
		| { title: string; copy: string }
		| { title: string; open: string }
		| ItemAction;

	export interface Section<T extends Item = Item> {
		title: string;
		items: T[];
	}

	export type List = { type?: 'list'; items: Item[] } | { type?: 'list'; sections: Section[] };

	/** Up to 50 items and 256 KiB of text. */
	export type SearchResult = Item[] | List;

	/**
	 * The entry module's `search` export: answers computed for each query. It runs as the user
	 * types, so it may only read, and write its own `storage`, and only once something is typed
	 * unless the manifest sets `emptyQuery`.
	 */
	export type Search = (query: string, ctx: SearchContext) => SearchResult | Promise<SearchResult>;

	/** What `items` receives. Dupin freezes it. */
	export interface ItemsContext {
		/** What the previous answer's `cursor` was, or null when Dupin holds no catalogue. */
		readonly cursor: string | null;
		/**
		 * `load` when Dupin holds nothing, `more` after an answer with `more: true`, `expired`
		 * after its `ttl` and `invalidated` after an action or event handler called
		 * `items.invalidate()`.
		 */
		readonly reason: 'load' | 'more' | 'expired' | 'invalidated';
	}

	interface CatalogueAnswer {
		/** Seconds, from 1 to 86,400, until Dupin asks again. Without it, it asks again only when invalidated. */
		ttl?: number;
		/** Up to 256 bytes, handed back as `ctx.cursor` next time. */
		cursor?: string;
		/** Ask again at once, with `cursor`, for the next part. */
		more?: boolean;
	}

	/** The whole catalogue, replacing what Dupin holds, or its first part with `more`. */
	export type CatalogueList = CatalogueAnswer &
		(
			| { type?: 'list'; items: CatalogueItem[] }
			| { type?: 'list'; sections: Section<CatalogueItem>[] }
		);

	/** Changes to the catalogue Dupin holds, by item `id`. */
	export interface CatalogueUpdate extends CatalogueAnswer {
		type: 'update';
		/** Items Dupin doesn't hold yet, after the others. */
		append?: CatalogueItem[];
		/** Items that take the place of the ones with their `id`. */
		replace?: CatalogueItem[];
		/** The `id` of each item to drop. */
		remove?: string[];
	}

	/** Up to 500 items, or 500 removals, and 1 MiB of text in one answer. */
	export type ItemsResult = CatalogueItem[] | CatalogueList | CatalogueUpdate;

	/**
	 * The entry module's `items` export: a catalogue Dupin holds and matches natively as the
	 * user types, asking again only on first use, after `ttl` or when an action or event handler
	 * invalidates it. At most 10,000 items and 8 MiB of text. It may use host services as
	 * `search` does: what reads, and `storage` writes too, but nothing that needs an action
	 * the user chose. With `defaultSearch`, it may run as the user types in the main search.
	 */
	export type Items = (ctx: ItemsContext) => ItemsResult | Promise<ItemsResult>;

	/** The host's catalogue controls. */
	export const items: {
		/** From an action or event handler: ask `items` again, with its cursor, before the next use. */
		invalidate(): void;
	};

	/** What an action may ask Dupin to do once it has run. */
	export type ActionEffect = { type: 'toast'; text: string };

	/**
	 * The entry module's `actions` export. A handler runs only because the user chose its item
	 * action, so it may also write, send and run programs. Dupin stops it after 30 seconds.
	 */
	export type Actions = Record<
		string,
		(
			data: Json,
			ctx: Readonly<Record<string, never>>,
		) => ActionEffect | void | Promise<ActionEffect | void>
	>;

	/** What `events.clipboard` receives for each copy, as `clipboard.read()` returns it. */
	export type ClipboardChange = Clip & { truncated?: true };

	/**
	 * The entry module's `events` export: handlers for what Dupin watches on the package's
	 * behalf, while it is on and holds the scope. Copies that arrive while one runs are coalesced
	 * to the newest. Handlers may read and use storage; they can't write the clipboard, send or
	 * run programs.
	 */
	export interface Events {
		clipboard?(change: ClipboardChange): void | Promise<void>;
	}

	/**
	 * Why a host request failed. `code` is stable: `needs-consent` (the user hasn't allowed the
	 * scope yet; Dupin shows them a request to allow it), `needs-activation` (only an action the
	 * user chose may do this), `disabled`, `denied`, `revoked`, `os-denied`, `undeclared`,
	 * `out-of-scope`, `quota-exceeded`, `invalid`, `too-many-calls`, `too-large`, `unavailable`,
	 * or the service's own code, such as `not-found` or `timeout`.
	 */
	export class DupinError extends Error {
		readonly name: 'DupinError';
		readonly code: string;
		/** The scope at stake, as grant files name it, such as `clipboard:read`. */
		readonly scope?: string;
		/** What allowing the scope lets the extension do, as Dupin shows it. */
		readonly description?: string;
		/** The macOS access the scope also needs, such as `Calendars`. */
		readonly permission?: string;
	}

	/** The `dupin` key of `package.json`, which Dupin reads as plain JSON before running code. */
	export interface Manifest {
		/** Shown on the keyword chip and in Settings, up to 80 characters. */
		title: string;
		/** Typed before a space to reach the extension. Up to 32 bytes, no spaces. */
		keyword: string;
		/** The module that exports `search` or `items`, inside the package. Defaults to `main.js`. */
		main?: `${string}.js` | `${string}.mjs` | `${string}.ts` | `${string}.mts` | `${string}.tsx`;
		/** List the `items` catalogue in the main search too, ranked with apps. The package never sees what is typed there. */
		defaultSearch?: boolean;
		/** Call `search` with an empty query as soon as the keyword is typed, for a package with something to show before anything is typed. */
		emptyQuery?: boolean;
		/** The most the extension may do. The user allows each scope separately. */
		capabilities?: Capabilities;
	}

	export type HttpMethod = 'GET' | 'HEAD' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';

	/** Each entry of a map or list is one scope the user allows on its own. */
	export interface Capabilities {
		/** Up to 32 `https://host[:port]` origins, lowercase, without a path or `:443`. */
		http?: { origin: `https://${string}`; methods: HttpMethod[] }[];
		/** Up to 16 named scopes; the user picks the file or folder each one is. */
		files?: Record<string, { kind: 'file' | 'directory'; access: ('read' | 'list')[] }>;
		/**
		 * Up to 16 named profiles: an argument template whose `{input}` placeholders take one line
		 * of text each, run with a program the user picks; or `@script:<name>` for one of the
		 * user's script commands. `output` asks to receive stdout and stderr.
		 */
		exec?: Record<string, { args?: string[]; output?: boolean }>;
		/** Encrypted records private to the package: 1 byte to 64 MiB, kept 1–3,650 days or until deleted. */
		storage?: { quotaBytes: number; retentionDays?: number };
		clipboard?: ('read' | 'write' | 'watch')[];
		dictionary?: ['lookup'];
		/** Each 0–366 days from today. */
		calendar?: { access: ['read']; pastDays: number; futureDays: number };
	}

	export interface PackageJson {
		/** The extension's id: 1–64 lowercase letters, digits or hyphens. No npm scopes. */
		name: string;
		/** 1–32 characters without spaces. */
		version: string;
		/** Include `dupin-extension`; GitHub discovery lists repositories with that topic. */
		keywords?: string[];
		/**
		 * The TypeScript or JavaScript entry `dupin ext build` bundles into `dupin.main`. Dupin
		 * itself ignores it. Leave it out when `dupin.main` is the source.
		 */
		source?: string;
		dupin: Manifest;
		[key: string]: unknown;
	}

	export interface HttpResponse {
		readonly ok: boolean;
		readonly status: number;
		/** The final URL, after redirects inside the declared origin. */
		readonly url: string;
		readonly redirected: boolean;
		/** Lowercase names, without cookies. */
		readonly headers: Readonly<Record<string, string>>;
		/** Whether the body was cut at 2 MiB. */
		readonly truncated: boolean;
		text(): Promise<string>;
		json(): Promise<unknown>;
	}

	/**
	 * `http` capability. GET and HEAD may run while the user types; other methods only in an
	 * action. No cookies or credentials are sent, and private addresses are refused.
	 */
	export const http: {
		fetch(
			url: string,
			init?: { method?: HttpMethod; headers?: Record<string, string>; body?: string },
		): Promise<HttpResponse>;
	};

	export interface FileEntry {
		name: string;
		kind: 'file' | 'directory' | 'symlink' | 'other';
		size?: number;
		/** Names the entry for Dupin's own actions; valid while it stays the same file. */
		handle?: string;
	}

	export interface FileContents {
		/** Lossy UTF-8 of the bytes read. */
		text: string;
		size: number;
		truncated: boolean;
		handle: string;
	}

	/** `files` capability. Paths are relative to the scope, with `/`; `..` is refused. */
	export const files: {
		/** At most `limit` bytes, 256 KiB by default and 2 MiB at most. */
		read(scope: string, path: string, options?: { limit?: number }): Promise<FileContents>;
		readText(scope: string, path: string, options?: { limit?: number }): Promise<string>;
		/** At most `limit` entries, 200 by default and 1,000 at most. */
		list(scope: string, path?: string, options?: { limit?: number }): Promise<FileEntry[]>;
	};

	export interface ExecResult {
		/** The exit code, or null when a signal or the timeout ended it. */
		readonly code: number | null;
		readonly signal: boolean;
		readonly timedOut: boolean;
		/** Present only when the profile declares `output` and the user allowed it. */
		readonly stdout?: string;
		readonly stderr?: string;
		readonly truncated?: boolean;
	}

	/** `exec` capability, only in an action. `profile` is a key of `exec`, such as `@script:open-in-zed`. */
	export const exec: {
		run(profile: string, inputs?: Record<string, string | number>): Promise<ExecResult>;
	};

	/** A blob stored with a record. The handle is opaque; pass it to `storage.readBlob`. */
	export interface StoredBlob {
		readonly handle: string;
		/** In bytes. */
		readonly size: number;
	}

	/** A record as `storage` returns it. Dupin freezes it, but not `data`. */
	export interface StoredRecord {
		/** Opaque and random; `storage.list` gives the order records were stored in. */
		readonly id: string;
		/** ISO 8601, UTC. */
		readonly createdAt: string;
		readonly data: Json;
		/** In the order they were given to `put`. */
		readonly blobs: readonly StoredBlob[];
	}

	export interface StoragePage {
		/** Newest first. */
		readonly records: readonly StoredRecord[];
		/** Pass as `before` for the next page; absent once this page reached the oldest. */
		readonly next?: string;
	}

	export interface StorageUsage {
		readonly entries: number;
		/** Each record's JSON plus each distinct blob, counted once however many records share it. */
		readonly bytes: number;
		readonly quotaBytes: number;
		readonly retentionDays?: number;
	}

	/**
	 * `storage` capability: records of JSON and the blobs they carry, encrypted at rest and private
	 * to the package. A record's JSON and blobs are at most 1 MiB together, with up to 16 blobs;
	 * Dupin keeps up to 10,000 records within the declared `quotaBytes`, and deletes records older
	 * than `retentionDays`. A `put` that doesn't fit fails with `quota-exceeded` and deletes
	 * nothing; one over 1 MiB fails with `too-large`.
	 */
	export const storage: {
		put(data: Json, options?: { blobs?: (ArrayBufferView | ArrayBuffer)[] }): Promise<StoredRecord>;
		/** `undefined` when no such record is stored. */
		get(id: string): Promise<StoredRecord | undefined>;
		/** Up to `limit` records (50 by default, 200 at most) older than `before`, newest first. */
		list(options?: { before?: string; limit?: number }): Promise<StoragePage>;
		/** Fails with `not-found` once no record carries the blob. */
		readBlob(handle: string): Promise<Uint8Array>;
		/** Whether the record was stored. Its blobs go once no other record carries them. */
		delete(id: string): Promise<boolean>;
		clear(): Promise<void>;
		usage(): Promise<StorageUsage>;
	};

	export type Clip =
		| { type: 'text'; text: string }
		| { type: 'url'; url: string }
		| { type: 'files'; files: { path: string; name: string }[] }
		| { type: 'image'; [key: string]: unknown };

	/** `clipboard` capability. Reading and writing happen only in an action. */
	export const clipboard: {
		read(): Promise<Clip | null>;
		readText(): Promise<string | undefined>;
		writeText(text: string): Promise<void>;
	};

	export interface DictionaryEntry {
		headword: string;
		[key: string]: unknown;
	}

	/** `dictionary` capability. */
	export const dictionary: {
		lookup(word: string): Promise<DictionaryEntry[]>;
	};

	export interface CalendarEvent {
		/** Names one occurrence of a recurring event. */
		readonly id: string;
		/** One line, up to 512 bytes, so it fits an item's `title` as it comes. */
		readonly title: string;
		/** ISO 8601, UTC. */
		readonly start: string;
		readonly end: string;
		readonly allDay: boolean;
		readonly calendar: {
			readonly name: string;
			/** `#rrggbb` in sRGB. */ readonly color: string | null;
		};
		readonly location: string | null;
		/** The event's own link, as its calendar holds it. */
		readonly url: string | null;
		/** Shows this occurrence in Calendar, as an `open` action of the `search` that read it. */
		readonly link: string | null;
		/** Its dates as this Mac's locale and time zone write them, since the guest has no `Intl`. */
		readonly labels: {
			/** The day it starts, such as `Wednesday 25 November`, with the year when it isn't this year. */
			readonly day: string;
			/** Only the time, such as `10:00 – 11:00`, or `All day`, when it falls within one day. */
			readonly time: string | null;
			/** Its dates and times, such as `23 Nov, 10:00 – 11:00`, with the year when it isn't this year. */
			readonly dates: string;
		};
	}

	/** `calendar` capability, inside the declared window around today. `to` is exclusive. */
	export const calendar: {
		/**
		 * Events overlapping the range, earliest start first, or latest with `order: 'latest'`:
		 * the first `limit` (100 by default, 500 at most) in that order whose title or location
		 * holds every word of `query`, in any order and case.
		 */
		events(range: {
			from: Date | string | number;
			to: Date | string | number;
			query?: string;
			limit?: number;
			order?: 'earliest' | 'latest';
		}): Promise<CalendarEvent[]>;
	};

	/**
	 * Drafts of what later API versions are planned to add, from docs/design/extension-platform.md.
	 * apiVersion 2 hosts provide none of it: the `dupin` module exports no such values and an
	 * item with these fields is rejected. Shapes may change before they ship.
	 */
	export namespace planned {
		type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

		/** Up to three text or tag accessories beside an item. */
		type Accessory = { text: string } | { tag: string };

		interface List {
			type: 'list';
			items?: Item[];
			sections?: Section[];
			/** Shown when there are no items. */
			emptyText?: string;
		}

		interface Detail extends DetailPane {
			type: 'detail';
			title: string;
			actions?: Action[];
		}

		type Field =
			| { type: 'text'; id: string; title: string; required?: boolean; default?: string }
			| { type: 'textarea'; id: string; title: string; required?: boolean; default?: string }
			| { type: 'checkbox'; id: string; title: string; default?: boolean }
			| {
					type: 'select';
					id: string;
					title: string;
					options: { value: string; title: string }[];
					default?: string;
			  };

		interface Form {
			type: 'form';
			title: string;
			fields: Field[];
			/** The named action that runs on submit with the field values. */
			submit: string;
		}

		type View = List | Detail | Form;

		interface ViewContext {
			readonly query: string;
			readonly preferences: Readonly<Record<string, string | boolean>>;
			readonly state: Json;
		}

		/** `views`: named handlers a `push` action opens. */
		type Views = Record<string, (ctx: ViewContext) => View | Promise<View>>;

		type Action =
			| { title: string; copy: string }
			| { title: string; open: string }
			| { title: string; push: { view: string; state?: Json } }
			| { title: string; run: string; data?: Json; destructive?: boolean };

		interface ItemsContext {
			readonly preferences: Readonly<Record<string, string | boolean>>;
		}

		type Effect =
			| { type: 'push'; view: string; state?: Json }
			| { type: 'pop' }
			| { type: 'refresh' }
			| { type: 'replaceState'; state: Json }
			| { type: 'toast'; text: string }
			| { type: 'fieldErrors'; errors: Record<string, string> };

		/** `actions` handlers that return an effect and see the view, including on form submit. */
		type Actions = Record<
			string,
			(data: Json, ctx: ViewContext) => Effect | void | Promise<Effect | void>
		>;
	}
}
