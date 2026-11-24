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
		/** Calls `actions[run](data)` in the package. `data` is up to 4 KiB of JSON. */
		| { title: string; run: string; data?: Json };

	export interface Section<T extends Item = Item> {
		title: string;
		items: T[];
	}

	export type List = { type?: 'list'; items: Item[] } | { type?: 'list'; sections: Section[] };

	/** Up to 50 items and 256 KiB of text. */
	export type SearchResult = Item[] | List;

	/** The entry module's `search` export: answers computed for each query. */
	export type Search = (query: string, ctx: SearchContext) => SearchResult | Promise<SearchResult>;

	/** What `items` receives. Dupin freezes it. */
	export interface ItemsContext {
		/** What the previous answer's `cursor` was, or null when Dupin holds no catalogue. */
		readonly cursor: string | null;
		/**
		 * `load` when Dupin holds nothing, `more` after an answer with `more: true`, `expired`
		 * after its `ttl` and `invalidated` after an action called `items.invalidate()`.
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
	 * user types, asking again only on first use, after `ttl` or when an action invalidates it.
	 * At most 10,000 items and 8 MiB of text.
	 */
	export type Items = (ctx: ItemsContext) => ItemsResult | Promise<ItemsResult>;

	/** The entry module's `actions` export: handlers an item's `run` action calls when chosen. */
	export type Actions = Record<string, (data: Json | undefined) => void | Promise<void>>;

	/** The host's catalogue controls. */
	export const items: {
		/** From an action handler: ask `items` again, with its cursor, before the next use. */
		invalidate(): void;
	};

	/** The `dupin` key of `package.json`, which Dupin reads as plain JSON before running code. */
	export interface Manifest {
		/** Shown on the keyword chip and in Settings, up to 80 characters. */
		title: string;
		/** Typed before a space to reach the extension. Up to 32 bytes, no spaces. */
		keyword: string;
		/** The module that exports `search` or `items`, inside the package. Defaults to `main.js`. */
		main?: `${string}.js` | `${string}.mjs`;
		/** List the `items` catalogue in the main search too, ranked with apps. The package never sees what is typed there. */
		defaultSearch?: boolean;
		/** Reserved: apiVersion 2 grants no capabilities, so any entry is an error. */
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
	 * apiVersion 2 hosts provide none of it: the `dupin` module exports no such values, an item
	 * with these fields is rejected, and a manifest that declares capabilities is an error.
	 * Shapes may change before they ship.
	 */
	export namespace v2 {
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
