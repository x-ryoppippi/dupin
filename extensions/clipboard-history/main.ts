import { clipboard, items as catalogue, storage } from 'dupin';
import type {
	Action,
	ActionEffect,
	CatalogueItem,
	ClipboardChange,
	HostImage,
	ItemsContext,
	ItemsResult,
	Metadata,
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

/**
 * A record of the package's storage: a copy, or a note that copies went, for `items`. `seq` is
 * one more than the newest record's before it, so `items` can tell what was stored since it
 * last listed even when a copy made again keeps its record's ID. The newest record always holds
 * the highest `seq` given, so the next one's stays above every one `items` has seen: a note that
 * is the newest is never deleted, and a note takes the place of a newest copy deleted unseen.
 */
type Stored = (
	| { kind: 'text'; text: string; source?: string }
	| { kind: 'image'; width: number; height: number; size: number; source?: string }
	| { kind: 'files'; files: CopiedFile[]; source?: string }
	| { kind: 'removed'; ids: string[] }
	| { kind: 'cleared' }
) & { copiedAt: number; seq?: number };

type Copy = Extract<Stored, { kind: 'text' | 'image' | 'files' }>;

/**
 * Where `items` is, in short keys to fit the 256 bytes Dupin keeps: `s` the newest `seq` it
 * listed, `o` the oldest copy it kept, `b` the bytes its rows cost by `cost`, and while it pages,
 * `n` the rows it expects, `l` the rows listed, `p` the parts sent and `c` storage's cursor.
 */
interface Cursor {
	s: number;
	o: number;
	b: number;
	n?: number;
	l?: number;
	p?: number;
	c?: string;
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
/** What Dupin takes in a title, a subtitle, a match text and a file path, in UTF-8 bytes. */
const TITLE_BYTES = 512;
const SUBTITLE_BYTES = 2048;
const MATCH_BYTES = 64 * 1024;
const FILE_BYTES = 4096;
const METADATA_BYTES = 512;
/** A title is cut no shorter than this to make room for the rows after it. */
const MIN_TITLE_BYTES = 48;
/**
 * What a row costs beyond its title, subtitle, match text and file, as Dupin counts a
 * catalogue's text: its ID and its actions' IDs, titles, handlers and data, with room to spare.
 */
const ROW_BYTES = 480;
/** What a typical row costs at least, reserved for each row a load has yet to list. */
const TYPICAL_MIN_ROW = ROW_BYTES + MIN_TITLE_BYTES + 32;
/** What any row costs at least: a page is never larger than its part has room for. */
const LARGEST_MIN_ROW = ROW_BYTES + MIN_TITLE_BYTES + SUBTITLE_BYTES + 2 * FILE_BYTES;
/** Dupin takes 1 MiB of text in one answer and holds 8 MiB of a catalogue. */
const PART_BYTES = 960 * 1024;
const CATALOGUE_BYTES = 8 * 1024 * 1024 - 256 * 1024;
/** What a load lists into, leaving the rest for copies prepended before the next load. */
const LOAD_BYTES = 6.5 * 1024 * 1024;
const PAGE = 200;
const PART_ROWS = 500;
/** Deletes one call makes, well inside the 256 host requests a call may make. */
const MAX_DELETES = 150;
const MAX_TRIM = 100;
/** Copies given up to make room for a new one, before it is skipped. */
const MAX_EVICTIONS = 32;
/** Retention is checked again an hour after the last load. */
const TTL = 3600;

function preferences(context: { preferences: Readonly<Record<string, unknown>> }): Preferences {
	return context.preferences as unknown as Preferences;
}

function data(record: StoredRecord): Stored {
	return record.data as unknown as Stored;
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

/**
 * The UTF-8 length of `text`; a lone surrogate becomes U+FFFD on its way to Dupin. ASCII is
 * counted by the regular expression engine, since copies run to 64 KiB.
 */
function utf8(text: string): number {
	const wide = text.replace(/[^\u0080-￿]+/g, '');
	let bytes = text.length - wide.length;
	for (let index = 0; index < wide.length; index++) {
		const code = wide.charCodeAt(index);
		if (code < 0x800) bytes += 2;
		else if (code >= 0xd800 && code < 0xdc00 && (wide.charCodeAt(index + 1) & 0xfc00) === 0xdc00) {
			bytes += 4;
			index++;
		} else bytes += 3;
	}
	return bytes;
}

/**
 * `text` in at most `bytes` UTF-8 bytes and `chars` characters, whole characters only, ending in
 * `more` when it is cut.
 */
function clip(text: string, bytes: number, chars = Infinity, more = ''): string {
	let end = text.length;
	if (chars < text.length) {
		let count = 0;
		end = 0;
		for (const char of text) {
			if (count === chars) break;
			end += char.length;
			count++;
		}
	}
	if (end === text.length && utf8(text) <= bytes) return text;
	const room = bytes - utf8(more);
	let low = 0;
	let high = end;
	while (low < high) {
		const middle = Math.ceil((low + high) / 2);
		if (utf8(text.slice(0, middle)) <= room) low = middle;
		else high = middle - 1;
	}
	if (low > 0 && (text.charCodeAt(low - 1) & 0xfc00) === 0xd800) low--;
	return low === 0 ? '' : `${text.slice(0, low)}${more}`;
}

/** The first words of `text`, on one line. */
function line(text: string): string {
	let shown = '';
	for (const word of text.split(/\s+/u)) {
		if (!word) continue;
		shown = shown ? `${shown} ${word}` : word;
		if (shown.length >= TITLE_CHARS && Array.from(shown).length >= TITLE_CHARS) break;
	}
	return shown;
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

/** What a copy's row shows and is found by, before it is cut to the room it has. */
interface View {
	title: string;
	subtitle: string;
	match: string;
	file?: string;
	/** Dupin draws it from the package's storage, as the row's icon and, selected, large. */
	image?: HostImage;
	metadata?: Metadata[];
}

/** Files Dupin draws as pictures, by their extension. */
const PICTURE = /\.(png|jpe?g|heic|heif|tiff?|gif|webp|bmp)$/iu;

function view(record: StoredRecord, copy: Copy): View {
	switch (copy.kind) {
		case 'text': {
			const count = lines(copy.text);
			return {
				title: line(copy.text),
				subtitle:
					count > 1
						? plural(count, 'line', 'lines')
						: plural(Array.from(copy.text).length, 'character', 'characters'),
				match: copy.text,
			};
		}
		case 'image': {
			const dimensions = `${copy.width}×${copy.height}`;
			const shown = size(copy.size);
			const blob = record.blobs[0]?.handle;
			return {
				title: `Image · ${dimensions} PNG`,
				subtitle: [shown, copy.source].filter(Boolean).join(' · '),
				match: [
					'image png',
					`${dimensions} ${copy.width}x${copy.height}`,
					shown,
					copy.source ?? '',
				].join('\n'),
				...(blob
					? {
							image: blob,
							metadata: details([
								['Dimensions', dimensions],
								['Size', shown],
								['Copied from', copy.source],
							]),
						}
					: {}),
			};
		}
		case 'files': {
			const [first] = copy.files;
			const file = utf8(first.path) <= FILE_BYTES ? first.path : undefined;
			const picture =
				file && first.handle && PICTURE.test(first.name)
					? { path: first.path, handle: first.handle }
					: undefined;
			const shown = first.size === undefined ? undefined : size(first.size);
			const alone = copy.files.length === 1;
			return {
				title: copy.files.map((file) => file.name).join(', '),
				subtitle: alone
					? [first.folder, shown].filter(Boolean).join(' · ')
					: plural(copy.files.length, 'file', 'files'),
				match: copy.files.map((file) => file.path).join('\n'),
				file,
				...(picture ? { image: picture } : {}),
				...(picture && alone
					? {
							metadata: details([
								['Folder', first.folder],
								['Size', shown],
							]),
						}
					: {}),
			};
		}
	}
}

/** A detail's metadata from the entries that have a value, each one line within its limit. */
function details(entries: [string, string | undefined][]): Metadata[] {
	return entries.flatMap(([label, text]) => {
		const shown = text && clip(text.replace(/\p{Cc}/gu, ' '), METADATA_BYTES);
		return shown ? [{ label, text: shown }] : [];
	});
}

/** What Dupin counts of an image a row or its detail names. */
function imageCost(image: HostImage | undefined): number {
	if (image === undefined) return 0;
	return typeof image === 'string' ? utf8(image) : utf8(image.path) + utf8(image.handle);
}

/** The detail shown while the row is selected: its image again, and its metadata. */
function detailCost(image: HostImage | undefined, metadata: Metadata[] | undefined): number {
	if (!metadata) return 0;
	return (
		imageCost(image) +
		metadata.reduce((sum, entry) => sum + utf8(entry.label) + utf8(entry.text), 0)
	);
}

/**
 * What a row costs whatever its title and match text: the file shows twice, once to reveal, and
 * an image once as the icon and once in the detail.
 */
function fixedCost(shown: View): number {
	return (
		ROW_BYTES +
		utf8(clip(shown.subtitle, SUBTITLE_BYTES)) +
		2 * utf8(shown.file ?? '') +
		imageCost(shown.image) +
		detailCost(shown.image, shown.metadata)
	);
}

function minCost(shown: View): number {
	return fixedCost(shown) + Math.min(MIN_TITLE_BYTES, utf8(shown.title));
}

function cost(item: CatalogueItem): number {
	return (
		ROW_BYTES +
		utf8(item.title) +
		utf8(item.subtitle ?? '') +
		utf8(item.matchText ?? '') +
		2 * utf8(item.file ?? '') +
		imageCost(item.image) +
		detailCost(item.detail?.image, item.detail?.metadata)
	);
}

/**
 * The row of a copy in at most `room` bytes by `cost`: its title is cut to fit, then its match
 * text, which only finds it. Null when not even its shortest title fits.
 */
function row(record: StoredRecord, copy: Copy, shown: View, room: number): CatalogueItem | null {
	const fixed = fixedCost(shown);
	if (room < minCost(shown)) return null;
	const title = clip(shown.title, Math.min(TITLE_BYTES, room - fixed), TITLE_CHARS, '…');
	const matchText = clip(shown.match, Math.min(MATCH_BYTES, room - fixed - utf8(title)));
	const ref = { id: record.id };
	const needsFile = copy.kind === 'files' ? { needsFile: true } : {};
	const actions: Action[] = [
		{ title: 'Copy', run: 'copy', data: ref, ...needsFile },
		{ title: 'Paste', run: 'paste', data: ref, alternate: true, ...needsFile },
	];
	if (shown.file) actions.push({ title: 'Reveal in Finder', reveal: shown.file });
	actions.push(
		{ title: 'Delete from History', run: 'remove', data: ref },
		{ title: 'Clear History', run: 'clear', confirm: 'Clear the whole clipboard history?' },
	);
	return {
		id: record.id,
		time: copy.copiedAt,
		title,
		subtitle: clip(shown.subtitle, SUBTITLE_BYTES),
		...(matchText ? { matchText } : {}),
		...(shown.file ? { file: shown.file } : {}),
		...(shown.image ? { image: shown.image } : {}),
		...(shown.image && shown.metadata
			? { detail: { image: shown.image, metadata: shown.metadata } }
			: {}),
		actions,
	};
}

function parse(cursor: string | null): Cursor | null {
	if (!cursor) return null;
	try {
		const parsed = JSON.parse(cursor) as Cursor;
		return typeof parsed.s === 'number' && typeof parsed.b === 'number' ? parsed : null;
	} catch {
		return null;
	}
}

function seqOf(record: StoredRecord): number {
	return data(record).seq ?? 0;
}

/**
 * The whole history, or the part after `from`, newest first, in parts Dupin takes and within
 * the text a catalogue may hold: the newest copies are found by all their text, and older ones
 * by less once room runs short. Copies the preferences no longer keep, and notes left for an
 * update, are deleted on the way, a bounded number per part.
 */
async function load(from: Cursor | null, prefs: Preferences): Promise<ItemsResult> {
	const cutoff = cutoffOf(prefs);
	const expected = from?.n ?? Math.min(prefs.maxEntries, (await storage.usage()).entries);
	let seq = from?.s ?? 0;
	let oldest = from?.o ?? Number.MAX_SAFE_INTEGER;
	let bytes = from?.b ?? 0;
	let listed = from?.l ?? 0;
	let before = from?.c;
	const part = (from?.p ?? 0) + 1;
	const rows: CatalogueItem[] = [];
	let partBytes = 0;
	let deletes = 0;
	let done = false;
	listing: for (;;) {
		const limit = Math.min(
			PAGE,
			PART_ROWS - rows.length,
			Math.floor((PART_BYTES - partBytes) / LARGEST_MIN_ROW),
		);
		if (limit <= 0) break;
		const page = await storage.list({ before, limit });
		const newest = before === undefined ? page.records[0]?.id : undefined;
		const shown: { record: StoredRecord; copy: Copy; view: View; min: number }[] = [];
		for (const record of page.records) {
			const stored = data(record);
			seq = Math.max(seq, stored.seq ?? 0);
			if (kept(stored, prefs, cutoff) && listed + shown.length < prefs.maxEntries) {
				const copyView = view(record, stored);
				shown.push({ record, copy: stored, view: copyView, min: minCost(copyView) });
			} else if (deletes < MAX_DELETES && (isCopy(stored) || record.id !== newest)) {
				await storage.delete(record.id);
				deletes++;
				if (record.id === newest) {
					await storage.put({
						kind: 'removed',
						ids: [],
						copiedAt: Date.now(),
						seq: stored.seq ?? 0,
					});
				}
			}
		}
		let pageReserve = shown.reduce((sum, entry) => sum + entry.min, 0);
		const later = Math.max(0, expected - listed - shown.length) * TYPICAL_MIN_ROW;
		for (const entry of shown) {
			pageReserve -= entry.min;
			const room = Math.min(
				PART_BYTES - partBytes - pageReserve,
				Math.max(entry.min, LOAD_BYTES - bytes - pageReserve - later),
				CATALOGUE_BYTES - bytes - pageReserve,
			);
			const item = row(entry.record, entry.copy, entry.view, room);
			if (!item) {
				done = true;
				break listing;
			}
			const spent = cost(item);
			rows.push(item);
			partBytes += spent;
			bytes += spent;
			listed++;
			oldest = Math.min(oldest, entry.copy.copiedAt);
		}
		before = page.next;
		if (!before) {
			done = true;
			break;
		}
	}
	const cursor: Cursor = done
		? { s: seq, o: oldest, b: bytes }
		: { s: seq, o: oldest, b: bytes, n: expected, l: listed, p: part, c: before };
	const answer = { cursor: JSON.stringify(cursor), more: !done, ttl: TTL };
	return from ? { ...answer, type: 'update', append: rows } : { ...answer, items: rows };
}

/** Copies stored since the catalogue last listed, and copies deleted since, or null to load. */
async function changes(cursor: Cursor, prefs: Preferences): Promise<ItemsResult | null> {
	const page = await storage.list({ limit: PAGE });
	if (page.records.length === 0) {
		return {
			items: [],
			cursor: JSON.stringify({ s: 0, o: Number.MAX_SAFE_INTEGER, b: 0 }),
			ttl: TTL,
		};
	}
	const fresh = page.records.filter((record) => seqOf(record) > cursor.s);
	if (fresh.length === page.records.length && page.next) return null;
	const cutoff = cutoffOf(prefs);
	const shown: { record: StoredRecord; copy: Copy; view: View; min: number }[] = [];
	const remove: string[] = [];
	const notes: string[] = [];
	let seq = cursor.s;
	for (const record of fresh) {
		const stored = data(record);
		seq = Math.max(seq, stored.seq ?? 0);
		if (stored.kind === 'cleared') return null;
		if (stored.kind === 'removed') {
			remove.push(...stored.ids);
			notes.push(record.id);
			continue;
		}
		remove.push(record.id);
		if (kept(stored, prefs, cutoff)) {
			const copyView = view(record, stored);
			shown.push({ record, copy: stored, view: copyView, min: minCost(copyView) });
		}
	}
	const removed = [...new Set(remove)];
	if (removed.length > PART_ROWS || shown.length > PART_ROWS) return null;
	const prepend: CatalogueItem[] = [];
	let reserve = shown.reduce((sum, entry) => sum + entry.min, 0);
	let used = 0;
	let oldest = cursor.o;
	for (const entry of shown) {
		reserve -= entry.min;
		const room = Math.min(PART_BYTES - used, CATALOGUE_BYTES - cursor.b - used) - reserve;
		const item = row(entry.record, entry.copy, entry.view, room);
		if (!item) return null;
		used += cost(item);
		prepend.push(item);
		oldest = Math.min(oldest, entry.copy.copiedAt);
	}
	for (const id of notes) if (id !== page.records[0].id) await storage.delete(id);
	const next: Cursor = { s: seq, o: oldest, b: cursor.b + used };
	return { type: 'update', prepend, remove: removed, cursor: JSON.stringify(next), ttl: TTL };
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
	if (ctx.reason === 'expired' && cursor && cursor.o >= cutoffOf(prefs)) {
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

async function newestSeq(): Promise<number> {
	const [newest] = (await storage.list({ limit: 1 })).records;
	return newest ? seqOf(newest) : 0;
}

/**
 * The copies to give up, in order, when storage has no room for `kind`: the oldest images
 * first, as they went from memory before, then for text or files the oldest of every other
 * copy. Text and files are never given up for an image.
 */
function evictable(records: StoredRecord[], kind: Copy['kind']): StoredRecord[] {
	const oldestFirst = [...records].reverse().filter((record) => isCopy(data(record)));
	const images = oldestFirst.filter((record) => data(record).kind === 'image');
	if (kind === 'image') return images;
	return [...images, ...oldestFirst.filter((record) => data(record).kind !== 'image')];
}

/** Deletes the oldest copies past `maxEntries` of `records`, newest first, and returns their IDs. */
async function trim(records: StoredRecord[], prefs: Preferences): Promise<string[]> {
	const extra = records
		.filter((record) => isCopy(data(record)))
		.slice(prefs.maxEntries, prefs.maxEntries + MAX_TRIM);
	for (const record of extra) await storage.delete(record.id);
	return extra.map((record) => record.id);
}

/** What a copy has given up so far to make room for itself, and what is left to give up. */
interface Room {
	kind: Copy['kind'];
	gone: Set<string>;
	records?: StoredRecord[];
	victims?: StoredRecord[];
}

function quotaExceeded(error: unknown): boolean {
	return (error as { code?: string }).code === 'quota-exceeded';
}

/** Runs `write`, giving up the next copy `room` allows each time storage has no room for it. */
async function withRoom<T>(room: Room, write: () => Promise<T>): Promise<T> {
	for (;;) {
		try {
			return await write();
		} catch (error) {
			if (!quotaExceeded(error) || room.gone.size >= MAX_EVICTIONS) throw error;
			room.records ??= await everything();
			room.victims ??= evictable(room.records, room.kind);
			const victim = room.victims.shift();
			if (!victim) throw error;
			await storage.delete(victim.id);
			room.gone.add(victim.id);
		}
	}
}

export const events = {
	async clipboard(
		change: ClipboardChange,
		ctx: { preferences: Readonly<Record<string, unknown>> },
	) {
		const prefs = preferences(ctx);
		const copy = stored(change, prefs);
		if (!copy) return;
		let seq = (await newestSeq()) + 1;
		const room: Room = { kind: copy.data.kind, gone: new Set() };
		try {
			let added: StoredRecord;
			try {
				added = await withRoom(room, () =>
					storage.put(
						{ ...copy.data, seq },
						{ key: copy.key, blobs: copy.blob ? [copy.blob] : [] },
					),
				);
			} catch (error) {
				// An image the copies older than it can't make room for is skipped, as it was in
				// memory.
				if (copy.data.kind === 'image' && quotaExceeded(error)) return;
				throw error;
			}
			seq++;
			let records = room.records && [
				added,
				...room.records.filter((record) => record.id !== added.id && !room.gone.has(record.id)),
			];
			if (!records && (await storage.usage()).entries > prefs.maxEntries)
				records = await everything();
			if (records) for (const id of await trim(records, prefs)) room.gone.add(id);
		} finally {
			if (room.gone.size > 0) {
				await withRoom(room, () =>
					storage.put({ kind: 'removed', ids: [...room.gone], copiedAt: Date.now(), seq }),
				);
			}
			catalogue.invalidate();
		}
	},
};

async function putBack(id: string): Promise<ActionEffect | null> {
	const record = await storage.get(id);
	const copy = record ? data(record) : undefined;
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
		const seq = (await newestSeq()) + 1;
		await storage.delete(id);
		await storage.put({ kind: 'removed', ids: [id], copiedAt: Date.now(), seq });
		catalogue.invalidate();
	},
	async clear(): Promise<ActionEffect> {
		// The marker follows every record the catalogue may have listed, even once they are gone.
		const seq = (await newestSeq()) + 1;
		await storage.clear();
		await storage.put({ kind: 'cleared', copiedAt: Date.now(), seq });
		catalogue.invalidate();
		return { type: 'toast', text: 'Cleared the clipboard history' };
	},
};
