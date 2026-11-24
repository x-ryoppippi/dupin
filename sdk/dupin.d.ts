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
	 * One row. Enter runs Open URL when `url` is set, otherwise Copy when `text` is set, otherwise
	 * the first of `actions`. Any other field fails the whole answer.
	 */
	export interface Item {
		/** One line, up to 512 bytes. */
		title: string;
		/** Up to 2,048 bytes. */
		subtitle?: string;
		/** Unique among the items, up to 128 bytes. Dupin numbers items without one. */
		id?: string;
		/** An `http` or `https` URL with a host, up to 2,048 bytes. */
		url?: string;
		/** Non-empty, up to 16 KiB. */
		text?: string;
		/** Named handlers of the module's `actions` export. Eight actions at most, `url` and `text` included. */
		actions?: ItemAction[];
	}

	/** Runs `actions[run](data, ctx)` when the user chooses it. */
	export interface ItemAction {
		/** One line, up to 512 bytes. */
		title: string;
		/** A key of the module's `actions` export, up to 64 bytes. */
		run: string;
		/** Passed to the handler as JSON: at most 4 KiB, nested at most 16 deep. */
		data?: Json;
	}

	export interface List {
		type?: 'list';
		items: Item[];
	}

	/** Up to 50 items and 256 KiB of text. */
	export type SearchResult = Item[] | List;

	/** The entry module's `search` export. It runs as the user types, so it may only read. */
	export type Search = (query: string, ctx: SearchContext) => SearchResult | Promise<SearchResult>;

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
		/** The module that exports `search`, inside the package. Defaults to `main.js`. */
		main?: `${string}.js` | `${string}.mjs` | `${string}.ts` | `${string}.mts` | `${string}.tsx`;
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
		/** Encrypted key-value data private to the package. */
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

	/** `storage` capability. Keys are 1–256 bytes; a value is at most 1 MiB of JSON. */
	export const storage: {
		get(key: string): Promise<Json | undefined>;
		set(key: string, value: Json): Promise<void>;
		delete(key: string): Promise<void>;
		keys(): Promise<string[]>;
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
		title: string;
		/** ISO 8601, UTC. */
		start: string;
		end: string;
		allDay: boolean;
		calendar: unknown;
		location?: string;
		[key: string]: unknown;
	}

	/** `calendar` capability, inside the declared window around today. `to` is exclusive. */
	export const calendar: {
		events(range: {
			from: Date | string | number;
			to: Date | string | number;
			limit?: number;
		}): Promise<CalendarEvent[]>;
	};

	/**
	 * Drafts of what later API versions are planned to add, from docs/design/extension-platform.md.
	 * apiVersion 2 hosts provide none of it: the `dupin` module exports no such values and an
	 * item with these fields is rejected. Shapes may change before they ship.
	 */
	export namespace v2 {
		type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

		/** `items(ctx)`: a list source. Dupin caches the rows and matches them as the user types. */
		type Items = (ctx: ItemsContext) => ListResult | Promise<ListResult>;

		interface ItemsContext {
			readonly limit: number;
			readonly preferences: Readonly<Record<string, string | boolean>>;
		}

		interface Item {
			title: string;
			subtitle?: string;
			id?: string;
			url?: string;
			text?: string;
			/** Extra words native matching considers. */
			keywords?: string[];
			/** A built-in icon name. */
			icon?: string;
			/** Up to three text or tag accessories. */
			accessories?: Accessory[];
			/** Shown in the selected-item Detail pane. */
			detail?: DetailPane;
			/** Up to eight. Enter runs the first. */
			actions?: Action[];
		}

		type Accessory = { text: string } | { tag: string };

		interface Section {
			title: string;
			items: Item[];
		}

		interface List {
			type: 'list';
			items?: Item[];
			sections?: Section[];
			/** Shown when there are no items. */
			emptyText?: string;
		}

		type ListResult = Item[] | List;

		interface Metadata {
			label: string;
			text: string;
		}

		interface DetailPane {
			/** Headings, paragraphs, emphasis, lists, inline and fenced code and HTTP(S) links. */
			markdown?: string;
			metadata?: Metadata[];
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

		type Effect =
			| { type: 'push'; view: string; state?: Json }
			| { type: 'pop' }
			| { type: 'refresh' }
			| { type: 'replaceState'; state: Json }
			| { type: 'toast'; text: string }
			| { type: 'fieldErrors'; errors: Record<string, string> };

		/** `actions`: named handlers that run on explicit activation, including form submit. */
		type Actions = Record<
			string,
			(data: Json, ctx: ViewContext) => Effect | void | Promise<Effect | void>
		>;
	}
}
