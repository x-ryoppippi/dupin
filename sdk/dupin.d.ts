// Types for Dupin extensions, apiVersion 1. Dupin runs JavaScript only and never reads this
// file; it is for editors and `tsc --noEmit`. `dupin ext new` copies it into each package.
// Docs: https://github.com/x-ryoppippi/dupin/blob/main/docs/features/extensions.md

declare module 'dupin' {
	/** The guest API this file describes. The host's `dupin` module exports the same value. */
	export const apiVersion: 1;

	/** What `search` receives after the query. Dupin freezes it. */
	export interface SearchContext {
		/** How many rows Dupin will show, at most 50. Rows beyond it are dropped. */
		readonly limit: number;
	}

	/**
	 * One row. Enter runs Open URL when `url` is set, otherwise Copy when `text` is set.
	 * Any other field fails the whole answer.
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
	}

	export interface List {
		type?: 'list';
		items: Item[];
	}

	/** Up to 50 items and 256 KiB of text. */
	export type SearchResult = Item[] | List;

	/** The entry module's `search` export. */
	export type Search = (query: string, ctx: SearchContext) => SearchResult | Promise<SearchResult>;

	/** The `dupin` key of `package.json`, which Dupin reads as plain JSON before running code. */
	export interface Manifest {
		/** Shown on the keyword chip and in Settings, up to 80 characters. */
		title: string;
		/** Typed before a space to reach the extension. Up to 32 bytes, no spaces. */
		keyword: string;
		/** The module that exports `search`, inside the package. Defaults to `main.js`. */
		main?: `${string}.js` | `${string}.mjs`;
		/** Reserved: apiVersion 1 grants no capabilities, so any entry is an error. */
		capabilities?: v2.Capabilities;
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

	/**
	 * Drafts of what later API versions are planned to add, from docs/design/extension-platform.md.
	 * apiVersion 1 hosts provide none of it: the `dupin` module exports no such values, an item
	 * with these fields is rejected, and a manifest that declares capabilities is an error.
	 * Shapes may change before they ship.
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

		/** The most a package may ask for. The user grants each scope separately. */
		interface Capabilities {
			/** HTTPS origins and methods. No ambient cookies or authentication. */
			http?: { origin: `https://${string}`; methods: HttpMethod[] }[];
			/** Named file or directory scopes the user picks. */
			files?: Record<string, { description: string; access: ('read' | 'list')[] }>;
			/**
			 * Named executable profiles, bound to an executable the user selects, or to a
			 * user-installed script command with an `@script:<name>` key.
			 */
			exec?: Record<string, { description: string; args?: string[]; returnsOutput?: boolean }>;
			/** Encrypted key-value storage private to the package. */
			storage?: true;
			clipboard?: { read?: boolean; write?: boolean };
			dictionary?: true;
			calendar?: { read: true };
		}

		type HttpMethod = 'GET' | 'HEAD' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';

		interface HttpResponse {
			readonly ok: boolean;
			readonly status: number;
			readonly headers: Readonly<Record<string, string>>;
			text(): Promise<string>;
			json(): Promise<unknown>;
		}

		/** Planned `import { http } from "dupin"`. */
		interface Http {
			fetch(
				url: string,
				init?: { method?: HttpMethod; headers?: Record<string, string>; body?: string },
			): Promise<HttpResponse>;
		}

		interface FileEntry {
			name: string;
			kind: 'file' | 'directory';
			size?: number;
		}

		/** Planned `import { files } from "dupin"`. Paths are relative to the named scope. */
		interface Files {
			readText(scope: string, path: string): Promise<string>;
			list(scope: string, path?: string): Promise<FileEntry[]>;
		}

		interface ExecResult {
			readonly code: number;
			/** Present only when the profile returns output and the user approved it. */
			readonly stdout?: string;
			readonly stderr?: string;
		}

		/** Planned `import { exec } from "dupin"`. Runs on explicit activation only. */
		interface Exec {
			run(profile: string, inputs?: Record<string, string>): Promise<ExecResult>;
		}

		/** Planned `import { storage } from "dupin"`. */
		interface Storage {
			get(key: string): Promise<Json | undefined>;
			set(key: string, value: Json): Promise<void>;
			delete(key: string): Promise<void>;
			keys(): Promise<string[]>;
		}

		/** Planned `import { clipboard } from "dupin"`. */
		interface Clipboard {
			readText(): Promise<string | undefined>;
			writeText(text: string): Promise<void>;
		}

		interface DictionaryEntry {
			headword: string;
			definition: string;
			dictionary: string;
		}

		/** Planned `import { dictionary } from "dupin"`. */
		interface Dictionary {
			lookup(word: string): Promise<DictionaryEntry[]>;
		}

		interface CalendarEvent {
			title: string;
			/** ISO 8601. */
			start: string;
			end: string;
			allDay: boolean;
			calendar: string;
			location?: string;
		}

		/** Planned `import { calendar } from "dupin"`. Times are ISO 8601. */
		interface Calendar {
			events(range: { from: string; to: string }): Promise<CalendarEvent[]>;
		}
	}
}
