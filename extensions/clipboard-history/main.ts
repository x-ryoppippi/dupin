import { clipboard, items as catalogue, storage } from 'dupin';
import type {
	Action,
	ActionEffect,
	CatalogueItem,
	ClipboardChange,
	ItemsContext,
	ItemsResult,
	StoredRecord,
} from 'dupin';

interface Preferences {
	retention: '1d' | '7d' | '30d' | '90d' | 'forever';
	includeImages: boolean;
	includeFiles: boolean;
	maxEntries: number;
}

type CopiedFile = {
	path: string;
	name: string;
	folder?: string;
	size?: number;
	handle?: string;
};

/** A record of the package's storage: a copy, or a note that copies went, for `items`. */
type Stored =
	| { kind: 'text'; text: string; copiedAt: number; source?: string }
	| {
			kind: 'image';
			width: number;
			height: number;
			size: number;
			copiedAt: number;
			source?: string;
	  }
	| { kind: 'files'; files: CopiedFile[]; copiedAt: number; source?: string }
	| { kind: 'removed'; ids: string[]; copiedAt: number }
	| { kind: 'cleared'; copiedAt: number };

type Copy = Extract<Stored, { kind: 'text' | 'image' | 'files' }>;

/**
 * Where `items` is: the newest record it listed, which storage lists before everything stored
 * after it, whatever the clock said; the oldest copy it kept; and while it pages, where.
 */
interface Cursor {
	newestId?: string;
	oldest: number;
	before?: string;
	part?: number;
	listed?: number;
}

const DAY_MS = 864e5;
const RETENTION_MS: Record<Preferences['retention'], number> = {
	'1d': DAY_MS,
	'7d': 7 * DAY_MS,
	'30d': 30 * DAY_MS,
	'90d': 90 * DAY_MS,
	forever: Infinity,
};
const TITLE_CHARS = 200;
const PAGE = 200;
const PART_ROWS = 500;
/** Deletes one call makes, well inside the 256 host requests a call may make. */
const MAX_DELETES = 150;
/** Parts one load asks for, inside the 21 Dupin takes. */
const MAX_PARTS = 18;
/** Retention is checked again an hour after the last load. */
const TTL = 3600;
/** Images given up to make room for a new one, before it is skipped. */
const MAX_EVICTIONS = 32;

function preferences(context: { preferences: Readonly<Record<string, unknown>> }): Preferences {
	return context.preferences as unknown as Preferences;
}

function isCopy(stored: Stored): stored is Copy {
	return stored.kind === 'text' || stored.kind === 'image' || stored.kind === 'files';
}

function kept(stored: Stored, prefs: Preferences, cutoff: number): stored is Copy {
	return (
		isCopy(stored) &&
		stored.copiedAt >= cutoff &&
		(stored.kind !== 'image' || prefs.includeImages) &&
		(stored.kind !== 'files' || prefs.includeFiles)
	);
}

function cutoffOf(prefs: Preferences): number {
	return Date.now() - RETENTION_MS[prefs.retention];
}

/** cyrb53: a 53-bit hash, so a key never repeats the content it names. */
function hash(text: string): string {
	let h1 = 0xdeadbeef;
	let h2 = 0x41c6ce57;
	for (let index = 0; index < text.length; index++) {
		const code = text.charCodeAt(index);
		h1 = Math.imul(h1 ^ code, 2654435761);
		h2 = Math.imul(h2 ^ code, 1597334677);
	}
	h1 = Math.imul(h1 ^ (h1 >>> 16), 2246822507) ^ Math.imul(h2 ^ (h2 >>> 13), 3266489909);
	h2 = Math.imul(h2 ^ (h2 >>> 16), 2246822507) ^ Math.imul(h1 ^ (h1 >>> 13), 3266489909);
	return (4294967296 * (2097151 & h2) + (h1 >>> 0)).toString(36);
}

/** The first words of `text`, on one line. */
function title(text: string): string {
	let line = '';
	for (const word of text.split(/\s+/u)) {
		if (!word) continue;
		line = line ? `${line} ${word}` : word;
		if (Array.from(line).length >= TITLE_CHARS) break;
	}
	const chars = Array.from(line);
	return chars.length > TITLE_CHARS ? `${chars.slice(0, TITLE_CHARS).join('')}…` : line;
}

function plural(count: number, one: string, many: string): string {
	return `${count} ${count === 1 ? one : many}`;
}

/** Bytes in decimal units, as Finder shows them. */
function size(bytes: number): string {
	const units = ['KB', 'MB', 'GB', 'TB'];
	if (bytes < 1000) return `${bytes} B`;
	let value = bytes / 1000;
	let unit = 0;
	while (value >= 999.95 && unit + 1 < units.length) {
		value /= 1000;
		unit++;
	}
	return `${value < 10 ? value.toFixed(1) : value.toFixed(0)} ${units[unit]}`;
}

function lines(text: string): number {
	const parts = text.split(/\r?\n/u);
	return parts.at(-1) === '' ? parts.length - 1 : parts.length;
}

function row(record: StoredRecord, copy: Copy): CatalogueItem {
	const data = { id: record.id };
	const needsFile = copy.kind === 'files' ? { needsFile: true } : {};
	const actions: Action[] = [
		{ title: 'Copy', run: 'copy', data, ...needsFile },
		{ title: 'Paste', run: 'paste', data, alternate: true, ...needsFile },
	];
	const item = { id: record.id, time: copy.copiedAt, actions };
	const ending: Action[] = [
		{ title: 'Delete from History', run: 'remove', data },
		{ title: 'Clear History', run: 'clear', confirm: 'Clear the whole clipboard history?' },
	];
	switch (copy.kind) {
		case 'text': {
			const count = lines(copy.text);
			return {
				...item,
				title: title(copy.text),
				subtitle:
					count > 1
						? plural(count, 'line', 'lines')
						: plural(Array.from(copy.text).length, 'character', 'characters'),
				matchText: copy.text,
				actions: [...actions, ...ending],
			};
		}
		case 'image': {
			const dimensions = `${copy.width}×${copy.height}`;
			const shown = size(copy.size);
			return {
				...item,
				title: `Image · ${dimensions} PNG`,
				subtitle: [shown, copy.source].filter(Boolean).join(' · '),
				matchText: [
					'image png',
					`${dimensions} ${copy.width}x${copy.height}`,
					shown,
					copy.source ?? '',
				].join('\n'),
				actions: [...actions, ...ending],
			};
		}
		case 'files': {
			const [first] = copy.files;
			const subtitle =
				copy.files.length === 1
					? [first.folder, first.size === undefined ? undefined : size(first.size)]
							.filter(Boolean)
							.join(' · ')
					: plural(copy.files.length, 'file', 'files');
			return {
				...item,
				title: copy.files.map((file) => file.name).join(', '),
				subtitle,
				matchText: copy.files.map((file) => file.path).join('\n'),
				file: first.path,
				actions: [...actions, { title: 'Reveal in Finder', reveal: first.path }, ...ending],
			};
		}
	}
}

function parse(cursor: string | null): Cursor | null {
	if (!cursor) return null;
	try {
		return JSON.parse(cursor) as Cursor;
	} catch {
		return null;
	}
}

/**
 * The whole history, or the part after `from`, newest first. Copies the preferences no longer
 * keep, and notes left for an update, are deleted on the way, a bounded number per part.
 */
async function load(from: Cursor | null, prefs: Preferences): Promise<ItemsResult> {
	const cutoff = cutoffOf(prefs);
	const part = (from?.part ?? 0) + 1;
	let before = from?.before;
	let newestId = from?.newestId;
	let oldest = from?.oldest ?? Number.MAX_SAFE_INTEGER;
	let listed = from?.listed ?? 0;
	const rows: CatalogueItem[] = [];
	let deletes = 0;
	let done = false;
	while (rows.length < PART_ROWS && deletes < MAX_DELETES) {
		const page = await storage.list({ before, limit: Math.min(PAGE, PART_ROWS - rows.length) });
		for (const record of page.records) {
			const stored = record.data as unknown as Stored;
			if (kept(stored, prefs, cutoff) && listed < prefs.maxEntries) {
				listed++;
				rows.push(row(record, stored));
				newestId ??= record.id;
				oldest = Math.min(oldest, stored.copiedAt);
			} else if (deletes < MAX_DELETES) {
				await storage.delete(record.id);
				deletes++;
			}
		}
		before = page.next;
		if (!before) {
			done = true;
			break;
		}
	}
	const more = !done && part < MAX_PARTS;
	const cursor: Cursor = more ? { newestId, oldest, before, part, listed } : { newestId, oldest };
	const answer = { cursor: JSON.stringify(cursor), more, ttl: TTL };
	return from ? { ...answer, type: 'update', append: rows } : { ...answer, items: rows };
}

/** Copies stored since the catalogue's newest, and copies deleted since, or null to load. */
async function changes(cursor: Cursor, prefs: Preferences): Promise<ItemsResult | null> {
	const page = await storage.list({ limit: PAGE });
	if (page.records.length === 0) {
		return {
			items: [],
			cursor: JSON.stringify({ oldest: Number.MAX_SAFE_INTEGER }),
			ttl: TTL,
		};
	}
	const cutoff = cutoffOf(prefs);
	const prepend: CatalogueItem[] = [];
	const remove: string[] = [];
	const notes: string[] = [];
	let newestId = cursor.newestId;
	let caughtUp = false;
	for (const record of page.records) {
		const stored = record.data as unknown as Stored;
		if (record.id === cursor.newestId) {
			caughtUp = true;
			break;
		}
		if (stored.kind === 'cleared') return null;
		if (stored.kind === 'removed') {
			remove.push(...stored.ids);
			notes.push(record.id);
			continue;
		}
		remove.push(record.id);
		if (kept(stored, prefs, cutoff)) {
			prepend.push(row(record, stored));
			if (prepend.length === 1) newestId = record.id;
		}
	}
	if (!caughtUp && page.next) return null;
	for (const id of notes) await storage.delete(id);
	const next: Cursor = { newestId, oldest: cursor.oldest };
	return {
		type: 'update',
		prepend,
		remove: [...new Set(remove)],
		cursor: JSON.stringify(next),
		ttl: TTL,
	};
}

export async function items(ctx: ItemsContext): Promise<ItemsResult> {
	const prefs = preferences(ctx);
	const cursor = parse(ctx.cursor);
	if (ctx.reason === 'more' && cursor) return load(cursor, prefs);
	if (ctx.reason === 'invalidated' && cursor) {
		const update = await changes(cursor, prefs);
		if (update) return update;
	}
	// Nothing has aged out since the catalogue was listed: keep it.
	if (ctx.reason === 'expired' && cursor && cursor.oldest >= cutoffOf(prefs)) {
		return { type: 'update', cursor: ctx.cursor ?? undefined, ttl: TTL };
	}
	return load(null, prefs);
}

/** What one change stores, or null for a copy the preferences don't keep. */
function stored(
	change: ClipboardChange,
	prefs: Preferences,
): { data: Copy; key: string; blob?: string } | null {
	const copiedAt = Date.now();
	const source = typeof change.source === 'string' ? change.source : undefined;
	switch (change.type) {
		case 'text':
		case 'url': {
			const text = change.type === 'text' ? change.text : change.url;
			// Text longer than clipboard history keeps is skipped, never kept in part.
			if (!text.trim() || change.truncated) return null;
			return {
				data: { kind: 'text', text, copiedAt, source },
				key: `text:${text.length}:${hash(text)}`,
			};
		}
		case 'image': {
			const { width, height, size, blob } = change as {
				width?: unknown;
				height?: unknown;
				size?: unknown;
				blob?: unknown;
			};
			if (!prefs.includeImages || typeof blob !== 'string') return null;
			if (typeof width !== 'number' || typeof height !== 'number' || typeof size !== 'number')
				return null;
			return {
				data: { kind: 'image', width, height, size, copiedAt, source },
				key: `image:${width}x${height}:${size}`,
				blob,
			};
		}
		case 'files': {
			if (!prefs.includeFiles || change.files.length === 0) return null;
			const files = change.files.map((file) => {
				const { path, name, folder, size, handle } = file as CopiedFile;
				return { path, name, folder, size, handle };
			});
			return {
				data: { kind: 'files', files, copiedAt, source },
				key: `files:${hash(files.map((file) => file.path).join('\n'))}`,
			};
		}
	}
}

/** Every stored record, newest first. */
async function everything(): Promise<StoredRecord[]> {
	const records: StoredRecord[] = [];
	let before: string | undefined;
	do {
		const page = await storage.list({ before, limit: PAGE });
		records.push(...page.records);
		before = page.next;
	} while (before);
	return records;
}

/** Deletes the oldest copies past `maxEntries`, and returns their IDs. */
async function trim(prefs: Preferences): Promise<string[]> {
	if ((await storage.usage()).entries <= prefs.maxEntries) return [];
	const copies = (await everything()).filter((record) => isCopy(record.data as unknown as Stored));
	const extra = copies.slice(prefs.maxEntries, prefs.maxEntries + MAX_DELETES);
	for (const record of extra) await storage.delete(record.id);
	return extra.map((record) => record.id);
}

/** Deletes the oldest image other than `keep`, and returns its ID. */
async function evictImage(keep: string | undefined): Promise<string | null> {
	const images = (await everything()).filter(
		(record) => (record.data as unknown as Stored).kind === 'image' && record.id !== keep,
	);
	const oldest = images.at(-1);
	if (!oldest) return null;
	await storage.delete(oldest.id);
	return oldest.id;
}

async function note(ids: string[]): Promise<void> {
	if (ids.length > 0) await storage.put({ kind: 'removed', ids, copiedAt: Date.now() });
}

export const events = {
	async clipboard(
		change: ClipboardChange,
		ctx: { preferences: Readonly<Record<string, unknown>> },
	) {
		const prefs = preferences(ctx);
		const copy = stored(change, prefs);
		if (!copy) return;
		const evicted: string[] = [];
		for (let attempt = 0; ; attempt++) {
			try {
				await storage.put(copy.data, { key: copy.key, blobs: copy.blob ? [copy.blob] : [] });
				break;
			} catch (error) {
				const full = (error as { code?: string }).code === 'quota-exceeded';
				// Images share the quota: the oldest go to make room for a new one, as they did
				// in memory. Text and files are never dropped to make room for images.
				const id =
					full && copy.data.kind === 'image' && attempt < MAX_EVICTIONS
						? await evictImage(undefined)
						: null;
				if (!id) {
					await note(evicted);
					throw error;
				}
				evicted.push(id);
			}
		}
		evicted.push(...(await trim(prefs)));
		await note(evicted);
		catalogue.invalidate();
	},
};

async function putBack(id: string): Promise<ActionEffect | null> {
	const record = await storage.get(id);
	const copy = record?.data as unknown as Stored | undefined;
	if (!record || !copy || !isCopy(copy))
		return { type: 'toast', text: 'That copy is no longer in the clipboard history.' };
	switch (copy.kind) {
		case 'text':
			await clipboard.write({ type: 'text', text: copy.text });
			break;
		case 'image':
			await clipboard.write({ type: 'image', blob: record.blobs[0].handle });
			break;
		case 'files':
			await clipboard.write({
				type: 'files',
				files: copy.files.map(({ path, handle }) => ({ path, handle: handle ?? '' })),
			});
			break;
	}
	return null;
}

export const actions = {
	async copy({ id }: { id: string }): Promise<ActionEffect> {
		return (await putBack(id)) ?? { type: 'hide' };
	},
	async paste({ id }: { id: string }): Promise<ActionEffect> {
		return (await putBack(id)) ?? { type: 'paste' };
	},
	async remove({ id }: { id: string }): Promise<void> {
		await storage.delete(id);
		await note([id]);
		catalogue.invalidate();
	},
	async clear(): Promise<ActionEffect> {
		await storage.clear();
		await storage.put({ kind: 'cleared', copiedAt: Date.now() });
		catalogue.invalidate();
		return { type: 'toast', text: 'Cleared the clipboard history' };
	},
};
