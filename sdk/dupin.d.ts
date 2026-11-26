// Types for Dupin extensions, apiVersion 4. Dupin never reads this file; it is for editors and
// `tsc --noEmit`. `dupin ext new` copies it into each package.
//
// Dupin runs a TypeScript `dupin.main` as written by erasing its types, without type-checking.
// Import these types with `import type { Item } from 'dupin'`: a type imported as a value stays
// in the module and fails to link. `enum`, namespaces with values, parameter properties,
// decorators and JSX need a build (`dupin ext build`), as do npm dependencies.
// Docs: https://github.com/x-ryoppippi/dupin/blob/main/docs/features/extension-api.md

declare module 'dupin' {
	/** The guest API this file describes. The host's `dupin` module exports the same value. */
	export const apiVersion: 4;

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
		/** An `http` or `https` URL with a host, a `dict://` URL or, from `search`, a calendar event's `link`; up to 2,048 bytes. */
		url?: string;
		/** Non-empty, up to 16 KiB. */
		text?: string;
		/**
		 * An image in the package (`.png`, `.jpg`, `.jpeg`, `.svg`, `.pdf` or `.icns`), relative to
		 * `package.json`, or an SF Symbol: `sf:` and its name, drawn in the colour of the row's
		 * text. Without one, a row in the main search shows the package's own `icon`; under the
		 * package's keyword it shows none, as the keyword's chip shows the package's.
		 */
		icon?: Icon;
		/** An image the package keeps, drawn scaled down as the row's icon in place of `icon`. */
		image?: HostImage;
		/** The heading the item is listed under; a section's items get its title. Up to 128 bytes. */
		section?: string;
		/** Shown while the item is selected. */
		detail?: DetailPane;
		/** Up to eight, counting the ones `url` and `text` add. */
		actions?: Action[];
		/** Up to three, shown at the row's end: `{ text }` as it is, `{ tag }` as a label. */
		accessories?: Accessory[];
		/**
		 * From `search` or a view: the row shows its title and this, one or two lines of up to
		 * 512 bytes together, as two values, as a calculation does.
		 */
		answer?: string;
	}

	/** One line of up to 64 bytes. */
	export type Accessory = { text: string } | { tag: string };

	/** Runs `actions[run](data, ctx)` when the user chooses it. */
	export interface ItemAction {
		/** One line, up to 512 bytes. */
		title: string;
		/** A key of the module's `actions` export: up to 64 letters, digits, `_` or `$`. */
		run: string;
		/** Passed to the handler as JSON, or `null` without it: at most 4 KiB, nested at most 16 deep. */
		data?: Json;
		/** ⌘Enter runs it. At most one action of an item. */
		alternate?: boolean;
		/** Dupin asks this first, one line, with the action's title as the button and Cancel selected. */
		confirm?: string;
		/** It acts on the item's `file`: while the file is gone, Dupin leaves it out. */
		needsFile?: boolean;
	}

	/** An item of an `items` catalogue, which may carry words that also find it. */
	export interface CatalogueItem extends Omit<Item, 'answer'> {
		/**
		 * Required, and kept with the item as the list changes: Dupin applies updates and
		 * remembers which items are picked by it.
		 */
		id: string;
		/** Up to 16 other names of up to 64 bytes, such as a short code, matched as the title is. */
		aliases?: string[];
		/** Up to 16 words of up to 64 bytes, matched below an equal match on the title. */
		keywords?: string[];
		/** With `"match": "words"`, more text, up to 64 KiB, that finds the item; it never shows. */
		matchText?: string;
		/** Milliseconds since the epoch the item is from: its subtitle starts with how long ago that was. */
		time?: number;
		/**
		 * The file the item stands for, by absolute path: it shows the file's Finder icon unless
		 * it has an `icon` or an `image`, and while the file is gone it shows as Unavailable,
		 * without its icon or image and the actions that need it.
		 */
		file?: string;
		/**
		 * Makes the item a web search, which takes no `url`, `text` or `actions`. The package's own
		 * keyword lists every web search for the query, and `webSearch.title` shows it; Enter
		 * opens the URL filled with the query. Only the bundled Web Search package's also reach
		 * the main search by their alias. Query matching never reaches the item.
		 */
		webSearch?: WebSearch;
	}

	/** Dupin fills `{query}` as the user types, without calling the package. */
	export interface WebSearch {
		/** Up to 32 ASCII letters, digits, `_` or `-`, matched case-sensitively. A keyword wins over it. */
		alias: string;
		/**
		 * An `http` or `https` URL of up to 4,096 bytes, with no credentials and one `{query}` after
		 * its host, which Dupin percent-encodes; or exactly `maps://?q={query}`, which opens Apple
		 * Maps and needs a query.
		 */
		url: string;
		/** The title once a query is typed: one line with one `{query}`, which the query replaces. */
		title: string;
		/** Offered, lowest first, when nothing else matches the main search: 0 to 1,000. */
		fallback?: number;
	}

	export interface Metadata {
		/** One line, up to 64 bytes. */
		label: string;
		/** One line, up to 512 bytes. */
		text: string;
	}

	/**
	 * An image Dupin draws from what the package keeps, by a handle and never by its bytes: a
	 * stored blob's `handle`, or a file its clipboard watch delivered, as `{ path, handle }` with
	 * the handle it came with. Dupin reads it from the package's own storage alone, decodes it
	 * only when it is PNG, TIFF, HEIC, HEIF or JPEG, up to 64 MiB and 64 megapixels, and scales
	 * it down off the panel's thread, only for the rows in view. While the package may not read
	 * its storage, the handle names nothing of its own, or the image is of another format or
	 * doesn't decode, the image is left out, and a detail keeps no space for it.
	 */
	export type HostImage = string | { path: string; handle: string };

	export interface DetailPane {
		/** Shown large above the metadata. */
		image?: HostImage;
		/** Up to 32 KiB: headings, paragraphs, emphasis, lists, inline and fenced code and HTTP(S) links. */
		markdown?: string;
		/** Up to 12 entries, above the Markdown. */
		metadata?: Metadata[];
	}

	/**
	 * `copy` and `paste` take 1 byte to 64 KiB of text. With `template`, Dupin fills `{date}`
	 * (`2026-01-31`), `{time}` (`09:05`), `{datetime}` and `{clipboard}` (the clipboard's text)
	 * when the action is chosen, so the package never sees the clipboard; other braces stay as
	 * written. Only a bundled package that replaces a built-in gets templates; any other's text
	 * is copied or pasted as written. `paste` needs the `clipboard: ['paste']` scope, and copies
	 * without it.
	 */
	export type Action =
		| { title: string; copy: string; template?: boolean }
		| { title: string; paste: string; template?: boolean }
		| { title: string; open: string }
		/** Shows the file at the absolute path selected in Finder; it opens nothing. */
		| { title: string; reveal: string }
		| PushAction
		| ItemAction;

	/**
	 * Opens `views[push.view](ctx)` on a page of its own above the list, titled with the
	 * action's title. Dupin holds `push.state` for the page, at most 16 KiB of JSON nested at
	 * most 16 deep, and hands it back as `ctx.state` to the view and to the actions chosen there.
	 * Esc, or ⌫ in an empty field, closes the page; at most eight are open at once.
	 */
	export interface PushAction {
		title: string;
		push: { view: string; state?: Json };
	}

	export interface Section<T extends Item = Item> {
		title: string;
		items: T[];
	}

	export type List = { type?: 'list'; items: Item[] } | { type?: 'list'; sections: Section[] };

	/** Up to 50 items and 256 KiB of text. */
	export type SearchResult = Item[] | List;

	/** What `views` and the actions chosen on their pages receive. Dupin freezes it. */
	export interface ViewContext {
		/** What is typed on the page; Dupin asks the view again as it changes. */
		readonly query: string;
		/** What the page holds: the push's `state`, or the last `replaceState`, or `null`. */
		readonly state: Json;
		readonly preferences: Preferences;
	}

	/** A list, as `search` returns one, with what the page shows while it has no items. */
	export type ListView = List & {
		/** One line of up to 256 bytes. */
		emptyText?: string;
	};

	/** A detail that fills the page, with its own actions, which Enter and ⌘K run. */
	export interface DetailView extends DetailPane {
		type: 'detail';
		/** One line, up to 512 bytes. */
		title: string;
		/** Up to eight. */
		actions?: Action[];
	}

	/**
	 * A form. Dupin keeps what is typed while the page is open, moves between the fields with
	 * Tab, checks that required fields hold more than spaces and, on ⌘Enter (or Enter outside a
	 * multi-line field), calls `actions[submit](values, ctx)` with each field's value by its
	 * `id`: a text field's text, a checkbox's boolean or a select's option `value`. Esc cancels.
	 */
	export interface Form {
		type: 'form';
		/** One line, up to 512 bytes. */
		title: string;
		/** 1 to 16, with different `id`s. */
		fields: Field[];
		/** A key of the module's `actions`. */
		submit: string;
		/** What the submit key says; `Submit` without it. Up to 128 bytes. */
		submitTitle?: string;
	}

	/**
	 * `id` is 1–64 letters, digits, `_` or `-`, and `title` one line of up to 128 bytes. A
	 * `text` field holds one line of up to 1 KiB and a `textarea` up to 16 KiB; a select's
	 * `default` is its first option's `value` without one.
	 */
	export type Field =
		| {
				type: 'text' | 'textarea';
				id: string;
				title: string;
				required?: boolean;
				default?: string;
				placeholder?: string;
		  }
		| { type: 'checkbox'; id: string; title: string; default?: boolean }
		| {
				type: 'select';
				id: string;
				title: string;
				/** 1 to 32, with different values, each one line of up to 128 bytes. */
				options: { value: string; title: string }[];
				default?: string;
		  };

	export type View = Item[] | ListView | DetailView | Form;

	/**
	 * The entry module's `views` export: the pages its `push` actions and effects open. A view
	 * runs as `search` does, again whenever the page's query or state changes, so it may only
	 * read, and write its own `storage`; each call starts with fresh module state, so keep what a
	 * page needs in its state. Dupin stops it after 2 seconds.
	 */
	export type Views = Record<string, (ctx: ViewContext) => View | Promise<View>>;

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
		readonly preferences: Preferences;
	}

	/**
	 * What the user chose for each of the manifest's `preferences`, or its default. The package
	 * Dupin shipped for a built-in that reads config.toml also finds that built-in's settings
	 * here, as Web Search finds `webSearches` and `webFallbacks`; a change asks `items` again
	 * with reason `load`. Dupin freezes it through and through.
	 */
	export type Preferences = Readonly<Record<string, Json>>;

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
		/** Items Dupin doesn't hold yet, before the others, in this order. */
		prepend?: CatalogueItem[];
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
	 * user types, asking again only on first use, after `ttl`, when an action or event handler
	 * invalidates it or when its preferences change. At most 10,000 items and 8 MiB of text. It
	 * may use host services as `search` does: what reads, and `storage` writes too, but nothing
	 * that needs an action the user chose. With `defaultSearch`, it may run as the user types in
	 * the main search.
	 */
	export type Items = (ctx: ItemsContext) => ItemsResult | Promise<ItemsResult>;

	/** The host's catalogue controls. */
	export const items: {
		/** From an action or event handler: ask `items` again, with its cursor, before the next use. */
		invalidate(): void;
	};

	/**
	 * What an action may ask Dupin to do once it has run: show one line; hide the panel as
	 * copying does; hide it and paste what the action put on the clipboard into the app in
	 * front (which needs Accessibility; without it, Dupin says how to allow it); open a view on
	 * a page above the one the action was chosen on, titled `title` or the extension's title;
	 * close that page; ask its view (or the search) again; give it new state, which asks its view
	 * again; or, from a form's submit, show what is wrong with its fields. The ones that move or
	 * change a page take a `toast` too, which Dupin shows once it has. An effect lands only while
	 * the page it was chosen on is still the one on screen. Toasts are one line of up to 512
	 * bytes, state at most 16 KiB of JSON and each field error one line of up to 256 bytes.
	 */
	export type ActionEffect =
		| { type: 'toast'; text: string }
		| { type: 'hide' }
		| { type: 'paste' }
		| { type: 'push'; view: string; title?: string; state?: Json; toast?: string }
		| { type: 'pop'; toast?: string }
		| { type: 'refresh'; toast?: string }
		| { type: 'replaceState'; state: Json; toast?: string }
		| { type: 'fieldErrors'; errors: Record<string, string> };

	/**
	 * What an action receives beside its data. On one of the package's pages, `query` and
	 * `state` are the page's; anywhere else they are `''` and `null`. Dupin freezes it.
	 */
	export type ActionContext = ViewContext;

	/**
	 * The entry module's `actions` export. A handler runs only because the user chose its item
	 * action or submitted a form, so it may also write, send and run programs. Dupin stops it
	 * after 30 seconds.
	 */
	export type Actions = Record<
		string,
		(data: Json, ctx: ActionContext) => ActionEffect | void | Promise<ActionEffect | void>
	>;

	/**
	 * What `events.clipboard` receives for each copy: as `clipboard.read()` returns it, except
	 * that an image's PNG stays with Dupin under `blob`, which `storage.put` stores as one of
	 * the record's blobs, and each file carries a `handle` that lets the package put it back on
	 * the clipboard while it may use its storage.
	 */
	export type ClipboardChange = (
		| { type: 'text'; text: string }
		| { type: 'url'; url: string }
		| { type: 'files'; files: CopiedFile[] }
		| {
				type: 'image';
				mediaType: 'image/png';
				width: number;
				height: number;
				size: number;
				blob: string;
		  }
	) & { truncated?: true; source?: string };

	export interface CopiedFile {
		path: string;
		name: string;
		/** The folder it is in, with the home folder written `~`. */
		folder?: string;
		/** Bytes, when the file could be measured as it was copied. */
		size?: number;
		/** Its uniform type identifier, such as `public.mpeg-4`. */
		kind?: string;
		handle?: string;
	}

	/**
	 * The entry module's `events` export: handlers for what Dupin watches on the package's
	 * behalf, while it is on and holds the scope. Copies that arrive while one runs are coalesced
	 * to the newest. Handlers may read and use storage; they can't write the clipboard, send or
	 * run programs.
	 */
	export interface Events {
		clipboard?(
			change: ClipboardChange,
			ctx: { readonly preferences: Preferences },
		): void | Promise<void>;
	}

	/**
	 * What a filter receives: the titles of one batch of a source's candidates, up to 1,000 of
	 * 512 bytes each, and for a matcher what was typed. Only a source `[advanced]` composes the
	 * filter onto hands it a batch, and only while the user allows `rows` for that source's kind
	 * (and `query`, for a matcher). Dupin freezes it.
	 */
	export interface FilterInput {
		/** Present for a matcher only. */
		readonly query?: string;
		readonly candidates: readonly { readonly title: string }[];
	}

	/** A candidate a matcher finds: its index in `candidates`. */
	export interface FilterMatch {
		index: number;
		/** Higher ranks first; a whole number from -1,000,000 to 1,000,000. Defaults to 0. */
		score?: number;
		/** The ascending character indices of its title that matched, which Dupin highlights. */
		positions?: number[];
	}

	/**
	 * The entry module's `filters` export, each declared in `dupin.provides.filters`. A matcher
	 * returns the candidates the query finds; a sorter returns every index of `candidates` once,
	 * in its order. A filter runs as the user types and reaches no host service: every request
	 * fails with `unavailable`. Past 250 ms, or on any failure, the source runs its own stage.
	 */
	export type Filters = Record<
		string,
		(
			input: FilterInput,
			ctx: { readonly preferences: Preferences },
		) => FilterMatch[] | number[] | Promise<FilterMatch[] | number[]>
	>;

	/**
	 * What an `actions` handler declared in `dupin.provides.kinds` receives as its data when the
	 * user chooses it on a row Dupin found: the one field it names, and only once they allow it.
	 */
	export type KindActionData =
		| { text: string }
		| { url: string }
		| { name: string }
		| { path: string };

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

	/**
	 * An image file in the package, relative to `package.json`, or `sf:` and an SF Symbol's name:
	 * 1 to 64 ASCII letters, digits and dots, such as `sf:moon.fill`. A name macOS has no symbol
	 * for draws nothing.
	 */
	export type Icon =
		| `sf:${string}`
		| `${string}.png`
		| `${string}.jpg`
		| `${string}.jpeg`
		| `${string}.svg`
		| `${string}.pdf`
		| `${string}.icns`;

	/** The `dupin` key of `package.json`, which Dupin reads as plain JSON before running code. */
	export interface Manifest {
		/** Shown on the keyword chip and in Settings, up to 80 characters. */
		title: string;
		/** Typed before a space to reach the extension. Up to 32 bytes, no spaces. */
		keyword: string;
		/**
		 * The package's own icon, on its keyword chip, in Settings and on each of its rows in the
		 * main search that names none (not under its keyword): an image file inside the package,
		 * as an item's `icon`, or `sf:` and an SF Symbol's name, such as `sf:puzzlepiece`.
		 */
		icon?: Icon;
		/** The module that exports `search` or `items`, inside the package. Defaults to `main.js`. */
		main?: `${string}.js` | `${string}.mjs` | `${string}.ts` | `${string}.mts` | `${string}.tsx`;
		/** List the `items` catalogue in the main search too, ranked with apps. The package never sees what is typed there. */
		defaultSearch?: boolean;
		/** Call `search` with an empty query as soon as the keyword is typed, for a package with something to show before anything is typed. */
		emptyQuery?: boolean;
		/**
		 * How the `items` catalogue is matched: `fuzzy` ranks items as apps are ranked, and
		 * `words` keeps the items that hold every word typed, ignoring case, in the catalogue's
		 * order, as a history is searched, and leaves them out of usage ranking.
		 */
		match?: 'fuzzy' | 'words';
		/** Up to 16 choices `ctx.preferences` hands every call, by name. */
		preferences?: Record<string, PreferenceDeclaration>;
		/** The most the extension may do. The user allows each scope separately. */
		capabilities?: Capabilities;
		/** Filters and kind actions `[advanced]` in config.toml may compose onto Dupin's own sources and kinds. */
		provides?: Provides;
	}

	/** Up to 8 filters, and 8 actions for each kind, named by 1–64 letters, digits or `_`. */
	export interface Provides {
		/** Each one an export of `filters`. A matcher also needs `capabilities.query`. */
		filters?: Record<string, { stage: 'matcher' | 'sorter' }>;
		/** Each one an export of `actions`, receiving the field it names, which `capabilities.rows` declares. */
		kinds?: {
			text?: Record<string, { field: 'text' }>;
			url?: Record<string, { field: 'url' }>;
			file?: Record<string, { field: 'name' | 'path' }>;
			app?: Record<string, { field: 'name' | 'path' }>;
		};
	}

	export type PreferenceDeclaration = { title: string } & (
		| { type: 'checkbox'; default: boolean }
		| { type: 'select'; options: string[]; default: string }
		| { type: 'number'; min: number; max: number; default: number }
	);

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
		/**
		 * Encrypted records private to the package: 1 byte to 64 MiB, kept 1–3,650 days or until
		 * deleted. `sync` asks for `storage:sync`: once the user grants it on a Mac, a package installed
		 * from Git keeps the same records on every Mac that syncs, the later write of a key winning.
		 */
		storage?: { quotaBytes: number; retentionDays?: number; sync?: boolean };
		/** `paste` lets a row's `paste` action paste into the front app; without it, Dupin copies. */
		clipboard?: ('read' | 'write' | 'watch' | 'paste')[];
		dictionary?: ['lookup'];
		calculator?: ['evaluate'];
		/**
		 * Add, change and delete the snippets the user stores in Dupin, from an action the user
		 * chose. The package Dupin ships for Snippets holds it from the start.
		 */
		snippets?: ['write'];
		/** Each 0–366 days from today. */
		calendar?: { access: ['read']; pastDays: number; futureDays: number };
		/** `read` lists displays and reads the focused window; `arrange` moves and resizes it. Both need Accessibility. */
		windows?: ('read' | 'arrange')[];
		/**
		 * The fields of each kind of row Dupin's own sources find that the package's filters and
		 * kind actions may receive. Another extension's rows never reach them.
		 */
		rows?: {
			text?: ('title' | 'text')[];
			url?: ('title' | 'url')[];
			file?: ('title' | 'name' | 'path')[];
			app?: ('title' | 'name' | 'path')[];
		};
		/**
		 * What is typed beyond the package's keyword: in the searches a matcher is composed into,
		 * and in the main search for the built-in the package Dupin ships answers there.
		 */
		query?: ['read'];
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
		/**
		 * A blob may be the `blob` of the latest clipboard change, whose image never crosses to
		 * the guest and may be up to 32 MiB. With `key`, up to 256 bytes, the record replaces the
		 * one stored for that key before, keeps its ID and lists as the newest.
		 */
		put(
			data: Json,
			options?: { blobs?: (ArrayBufferView | ArrayBuffer | string)[]; key?: string },
		): Promise<StoredRecord>;
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
		| { type: 'files'; files: CopiedFile[] }
		/** `data` is the PNG in Base64, up to 2 MiB. */
		| { type: 'image'; mediaType: 'image/png'; width: number; height: number; data: string };

	/** `clipboard` capability. Reading and writing happen only in an action. */
	export const clipboard: {
		read(): Promise<Clip | null>;
		readText(): Promise<string | undefined>;
		writeText(text: string): Promise<void>;
		/**
		 * Text; an image the package stored, by its blob's handle; or files a watch delivered,
		 * by the handles they came with, which must still be there.
		 */
		write(
			clip:
				| { type: 'text'; text: string }
				| { type: 'image'; blob: string }
				| { type: 'files'; files: { path: string; handle: string }[] },
		): Promise<void>;
	};

	/** One sense: its text, its examples and, one level deep, its sub-senses. */
	export interface DictionarySense {
		definition: string;
		examples: string[];
		senses: DictionarySense[];
	}

	/** The senses under one part of speech, or under none before the first. */
	export interface DictionaryGroup {
		partOfSpeech: string | null;
		/** What comes before the numbered senses, such as inflections. */
		note: string | null;
		senses: DictionarySense[];
	}

	/** A labeled part such as Phrases or Origin, as plain text, one paragraph or item a line. */
	export interface DictionarySection {
		title: string;
		text: string;
	}

	/** Each structured text is at most 4 KiB; `truncated` says whether anything was cut. */
	export interface DictionaryEntry {
		headword: string;
		/** The dictionary the entry came from; `null` on macOS, which doesn't say. */
		dictionary: string | null;
		pronunciation: string | null;
		/** The definition as the dictionary wrote it, up to 16 KiB. */
		text: string;
		/**
		 * The entry as Dupin lays it out, up to 32 KiB of Markdown for an item's `detail`: each
		 * part of speech under a `###` heading with its numbered senses and examples in italics,
		 * and each labeled part under a `##` heading. The headword and pronunciation are left
		 * to the item's title and metadata.
		 */
		markdown: string;
		groups: DictionaryGroup[];
		sections: DictionarySection[];
		truncated: boolean;
	}

	/** `dictionary` capability. */
	export const dictionary: {
		lookup(word: string): Promise<DictionaryEntry[]>;
	};

	/** One part of a calculation's answer, in order. */
	export interface CalculationPart {
		/** `ident` is a unit or currency, such as ` mi` or `€`, with any space beside it. */
		kind: 'number' | 'ident' | 'other';
		text: string;
	}

	/** What a calculation came to, as Dupin's calculator writes it. */
	export interface Calculation {
		/** Such as `approx. 3.1068559612 mi`, up to 256 bytes. */
		result: string;
		/** Its first number alone, such as `3.1068559612`. */
		number: string;
		parts: CalculationPart[];
		/**
		 * Present when it converted a currency, which it does only while currency conversion is
		 * on: the European Central Bank's reference rates it used, their date (`YYYY-MM-DD`) and
		 * whether Dupin couldn't refresh them in the past 12 hours; or that it had no rates, or
		 * none for one currency, so `result` is not to be shown.
		 */
		exchange?:
			| { status: 'ok'; date: string; stale: boolean; currencies: string[] }
			| { status: 'unavailable' }
			| { status: 'missing'; currency: string };
	}

	/** `calculator` capability. */
	export const calculator: {
		/**
		 * Works out arithmetic, units and currencies within 50 ms. An empty expression, one over 512
		 * bytes or one with a control character such as a line break fails with `invalid`. `null`
		 * when it isn't something the calculator answers: unbalanced or deeply nested brackets,
		 * characters it doesn't take, an error or an answer longer than 256 bytes.
		 */
		evaluate(expression: string): Promise<Calculation | null>;
	};

	/** What stops a snippet from being stored, one line per field; no field when it was stored. */
	export interface SnippetProblems {
		readonly keyword?: string;
		readonly name?: string;
		readonly text?: string;
	}

	/**
	 * `snippets` capability: the snippets the user stores in Dupin, encrypted, beside the ones
	 * config.toml declares, which no package changes. Only an action the user chose may call it.
	 */
	export const snippets: {
		/**
		 * Stores the snippet in place of the stored one keyed `replaces`, or as a new one without
		 * it. Dupin checks it as config.toml's: a keyword of 1–32 bytes without spaces that no
		 * other snippet has, a one-line name of up to 80 characters or none, and 1 byte to 64 KiB
		 * of text; a keyword or name over 1 KiB, or text over 128 KiB, fails with `invalid`.
		 * Resolves to what is wrong, by field, which a form's `fieldErrors` can show as it is.
		 */
		save(
			snippet: { keyword: string; name?: string | null; text: string },
			options?: { replaces?: string | null },
		): Promise<SnippetProblems>;
		/** Deletes the stored snippet `keyword`; `false` when there was none. */
		delete(keyword: string): Promise<boolean>;
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
	 * A rectangle in points. Every display shares one space: the origin is the primary display's
	 * top-left corner and `y` grows downwards.
	 */
	export interface WindowFrame {
		x: number;
		y: number;
		width: number;
		height: number;
	}

	export interface WindowDisplay {
		readonly frame: Readonly<WindowFrame>;
		/** The part windows may use, outside the menu bar and the Dock. */
		readonly visibleFrame: Readonly<WindowFrame>;
		/** Pixels per point. */
		readonly scale: number;
	}

	export interface FocusedWindow {
		readonly frame: Readonly<WindowFrame>;
		/** The app that was in front when the panel opened. */
		readonly app: { readonly name: string; readonly bundleId: string | null };
	}

	/** `windows` capability: the focused window of the app that was in front when the panel opened. */
	export const windows: {
		/** Every display, the primary one first. Needs `read`. */
		displays(): Promise<WindowDisplay[]>;
		/** That window, or `null` when the app has none. Needs `read`. */
		focused(): Promise<FocusedWindow | null>;
		/**
		 * Moves and resizes that window and resolves to the frame it took, which the app may
		 * constrain. The frame must lie at least partly on a display. Needs `arrange` and an action
		 * the user chose.
		 */
		setFrame(frame: WindowFrame): Promise<WindowFrame>;
	};
}
