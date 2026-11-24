import type { Action, CatalogueItem, ItemsContext, ItemsResult } from 'dupin';

/** One snippet from config.toml, as Dupin hands over the valid ones in keyword order. */
interface Snippet {
	keyword: string;
	name: string | null;
	text: string;
}

const PREVIEW_CHARS = 80;
const PART_ROWS = 500;
const PART_BYTES = 1024 * 1024;
// Unicode's White_Space, which the built-in collapsed; JavaScript's `\s` also takes U+FEFF.
const WHITESPACE = new Set([
	0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x20, 0x85, 0xa0, 0x1680, 0x2000, 0x2001, 0x2002, 0x2003, 0x2004,
	0x2005, 0x2006, 0x2007, 0x2008, 0x2009, 0x200a, 0x2028, 0x2029, 0x202f, 0x205f, 0x3000,
]);

function snippets(ctx: ItemsContext): Snippet[] {
	const listed = ctx.preferences.snippets;
	return Array.isArray(listed) ? (listed as unknown as Snippet[]) : [];
}

/** The text on one line, whitespace runs collapsed, cut to fit a subtitle. */
function preview(text: string): string {
	const chars: string[] = [];
	let space = false;
	for (const char of text) {
		if (WHITESPACE.has(char.codePointAt(0) ?? 0)) {
			space = chars.length > 0;
			continue;
		}
		if (space) chars.push(' ');
		space = false;
		chars.push(char);
		if (chars.length > PREVIEW_CHARS) break;
	}
	return chars.length > PREVIEW_CHARS
		? `${chars.slice(0, PREVIEW_CHARS - 1).join('')}…`
		: chars.join('');
}

function utf8Bytes(text: string): number {
	let bytes = 0;
	for (const char of text) {
		const code = char.codePointAt(0) ?? 0;
		bytes += code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
	}
	return bytes;
}

function item(snippet: Snippet, paste: boolean): CatalogueItem {
	const title = snippet.name ?? snippet.keyword;
	const copy: Action = { title: 'Copy to Clipboard', copy: snippet.text, template: true };
	const actions: Action[] = paste
		? [{ title: 'Paste into Front App', paste: snippet.text, template: true }, copy]
		: [copy];
	return {
		id: `snippet:${snippet.keyword}`,
		title,
		subtitle: `${snippet.keyword}  ·  ${preview(snippet.text)}`,
		// A keyword that is also the title would only match where the title already does.
		aliases: snippet.keyword === title ? [] : [snippet.keyword],
		actions,
	};
}

/** The text Dupin weighs an item by against an answer's limit. */
function weight(row: CatalogueItem): number {
	const fields = [row.id, row.title, row.subtitle ?? '', ...(row.aliases ?? [])];
	for (const action of row.actions ?? []) {
		fields.push(action.title);
		if ('copy' in action) fields.push(action.copy);
		if ('paste' in action) fields.push(action.paste);
	}
	return fields.reduce((sum, field) => sum + utf8Bytes(field), 0);
}

export function items(ctx: ItemsContext): ItemsResult {
	const listed = snippets(ctx);
	const paste = ctx.preferences.paste === true;
	const start = ctx.reason === 'more' ? Number(ctx.cursor) : 0;
	const part: CatalogueItem[] = [];
	let bytes = 0;
	let next = start;
	while (next < listed.length && part.length < PART_ROWS) {
		const row = item(listed[next], paste);
		const size = weight(row);
		if (part.length > 0 && bytes + size > PART_BYTES) break;
		part.push(row);
		bytes += size;
		next += 1;
	}
	const more = next < listed.length;
	return start === 0
		? { items: part, cursor: String(next), more }
		: { type: 'update', append: part, cursor: String(next), more };
}
