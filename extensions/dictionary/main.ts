import { dictionary, DupinError } from 'dupin';
import type { DictionaryEntry, Item, SearchContext } from 'dupin';

// What Dupin accepts of a row, in UTF-8 bytes.
const MAX_ID_BYTES = 128;
const MAX_TITLE_BYTES = 512;
const MAX_SUBTITLE_BYTES = 2048;
const MAX_METADATA_BYTES = 512;
const MAX_URL_BYTES = 2048;

// A term longer than a lookup takes has no definition, as in Dictionary; Dupin itself shows the
// request to turn the extension on or allow the lookup.
const QUIET = new Set(['invalid', 'needs-consent', 'disabled']);

export async function search(query: string, ctx: SearchContext): Promise<Item[]> {
	const term = query.trim();
	let entries: DictionaryEntry[];
	try {
		entries = await dictionary.lookup(term);
	} catch (error) {
		if (error instanceof DupinError && QUIET.has(error.code)) return [];
		throw error;
	}
	return entries.slice(0, ctx.limit).map((entry, index) => row(term, entry, index));
}

function row(term: string, entry: DictionaryEntry, index: number): Item {
	const id = `dictionary:${cut(term, MAX_ID_BYTES - 16)}`;
	const first = entry.groups.flatMap((group) => group.senses)[0];
	const url = `dict://${encode(term)}`;
	return {
		id: index === 0 ? id : `${id}#${index}`,
		title: oneLine(entry.headword, MAX_TITLE_BYTES) || term,
		subtitle: cut(first?.definition ?? '', MAX_SUBTITLE_BYTES),
		detail: {
			markdown: entry.markdown,
			metadata: entry.pronunciation
				? [{ label: 'Pronunciation', text: oneLine(entry.pronunciation, MAX_METADATA_BYTES) }]
				: [],
		},
		actions: [
			...(url.length <= MAX_URL_BYTES ? [{ title: 'Open in Dictionary', open: url }] : []),
			...(entry.text ? [{ title: 'Copy Definition', copy: entry.text }] : []),
		],
	};
}

/** As Dictionary's own `dict://` links encode a word: every byte but unreserved ones. */
function encode(term: string): string {
	return encodeURIComponent(term).replace(
		/[!'()*]/g,
		(c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`,
	);
}

function oneLine(text: string, maxBytes: number): string {
	return cut(text.replace(/\s+/g, ' ').trim(), maxBytes);
}

/** `text` cut to at most `maxBytes` of UTF-8, on a character boundary. */
function cut(text: string, maxBytes: number): string {
	// A UTF-16 unit is at most 3 bytes of UTF-8, so most text needs no counting.
	if (text.length * 3 <= maxBytes) return text;
	let total = 0;
	let end = 0;
	for (const char of text) {
		const code = char.codePointAt(0) ?? 0;
		total += code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
		if (total > maxBytes) break;
		end += char.length;
	}
	return text.slice(0, end);
}
