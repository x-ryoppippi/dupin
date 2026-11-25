import { snippets as library } from 'dupin';
import type {
	Action,
	ActionContext,
	ActionEffect,
	CatalogueItem,
	ItemsContext,
	ItemsResult,
	Json,
	View,
	ViewContext,
} from 'dupin';

/**
 * One snippet as Dupin hands over the valid ones in keyword order: config.toml's, which only
 * that file changes, and the ones stored in Dupin, which this package edits.
 */
interface Snippet {
	keyword: string;
	name: string | null;
	text: string;
	source: 'config' | 'stored';
}

const PREVIEW_CHARS = 80;
const PART_ROWS = 500;
const PART_BYTES = 1024 * 1024;
/** The most a form's multi-line field holds; a longer snippet is edited in Settings. */
const FORM_TEXT_BYTES = 16 * 1024;
const ADD_ID = 'add-snippet';
/** As many snippets as Dupin lists, which a catalogue holds with no room for the row that adds one. */
const MAX_SNIPPETS = 10_000;
// Unicode's White_Space, which the built-in collapsed; JavaScript's `\s` also takes U+FEFF.
const WHITESPACE = new Set([
	0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x20, 0x85, 0xa0, 0x1680, 0x2000, 0x2001, 0x2002, 0x2003, 0x2004,
	0x2005, 0x2006, 0x2007, 0x2008, 0x2009, 0x200a, 0x2028, 0x2029, 0x202f, 0x205f, 0x3000,
]);

function snippets(ctx: { preferences: ItemsContext['preferences'] }): Snippet[] {
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

const ADD: Action = { title: 'Add Snippet', push: { view: 'edit' } };

function item(snippet: Snippet, paste: boolean, full: boolean): CatalogueItem {
	const title = snippet.name ?? snippet.keyword;
	const copy: Action = { title: 'Copy to Clipboard', copy: snippet.text, template: true };
	const actions: Action[] = paste
		? [{ title: 'Paste into Front App', paste: snippet.text, template: true }, copy]
		: [copy];
	if (snippet.source === 'stored') {
		if (utf8Bytes(snippet.text) <= FORM_TEXT_BYTES) {
			actions.push({
				title: 'Edit Snippet',
				push: { view: 'edit', state: { keyword: snippet.keyword } },
			});
		}
		actions.push({
			title: 'Delete Snippet',
			run: 'remove',
			data: { keyword: snippet.keyword },
			confirm: `Delete the snippet “${title}”?`,
		});
	}
	if (!full) actions.push(ADD);
	return {
		id: `snippet:${snippet.keyword}`,
		title,
		subtitle: `${snippet.keyword}  ·  ${preview(snippet.text)}`,
		// A keyword that is also the title would only match where the title already does.
		aliases: snippet.keyword === title ? [] : [snippet.keyword],
		accessories: snippet.source === 'config' ? [{ tag: 'config.toml' }] : [],
		actions,
	};
}

/** The text Dupin weighs an item by against an answer's limit. */
function weight(row: CatalogueItem): number {
	const fields = [row.id, row.title, row.subtitle ?? '', ...(row.aliases ?? [])];
	for (const accessory of row.accessories ?? []) {
		fields.push('tag' in accessory ? accessory.tag : accessory.text);
	}
	for (const action of row.actions ?? []) {
		fields.push(action.title);
		if ('copy' in action) fields.push(action.copy);
		if ('paste' in action) fields.push(action.paste);
		if ('push' in action) fields.push(JSON.stringify(action.push));
		if ('run' in action)
			fields.push(action.run, JSON.stringify(action.data ?? null), action.confirm ?? '');
	}
	return fields.reduce((sum, field) => sum + utf8Bytes(field), 0);
}

export function items(ctx: ItemsContext): ItemsResult {
	const listed = snippets(ctx);
	const paste = ctx.preferences.paste === true;
	const start = ctx.reason === 'more' ? Number(ctx.cursor) : 0;
	const part: CatalogueItem[] = [];
	const full = listed.length >= MAX_SNIPPETS;
	if (start === 0 && !full) {
		part.push({
			id: ADD_ID,
			title: 'Add Snippet',
			subtitle: 'Store a new snippet in Dupin, encrypted',
			icon: 'sf:plus',
			actions: [ADD],
		});
	}
	let bytes = 0;
	let next = start;
	while (next < listed.length && part.length < PART_ROWS) {
		const row = item(listed[next], paste, full);
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

/** The keyword of the stored snippet a page edits, or `null` on a page that adds one. */
function editing(ctx: ViewContext): string | null {
	const state = ctx.state;
	if (state === null || typeof state !== 'object' || Array.isArray(state)) return null;
	const keyword = (state as Record<string, Json>).keyword;
	return typeof keyword === 'string' ? keyword : null;
}

export const views = {
	edit(ctx: ViewContext): View {
		const keyword = editing(ctx);
		const snippet =
			keyword === null
				? undefined
				: snippets(ctx).find((listed) => listed.source === 'stored' && listed.keyword === keyword);
		if (keyword !== null && snippet === undefined) {
			return { type: 'list', items: [], emptyText: 'This snippet is no longer stored.' };
		}
		return {
			type: 'form',
			title: snippet === undefined ? 'New Snippet' : `Edit ${snippet.name ?? snippet.keyword}`,
			submit: 'save',
			submitTitle: 'Save Snippet',
			fields: [
				{
					type: 'text',
					id: 'name',
					title: 'Name',
					default: snippet?.name ?? '',
					placeholder: 'Shown in place of the keyword',
				},
				{
					type: 'text',
					id: 'keyword',
					title: 'Keyword',
					required: true,
					default: snippet?.keyword ?? '',
					placeholder: 'Finds it, without spaces',
				},
				{
					type: 'textarea',
					id: 'text',
					title: 'Text',
					required: true,
					default: snippet?.text ?? '',
					placeholder: 'Fills in {date}, {time}, {datetime} and {clipboard}',
				},
			],
		};
	},
};

export const actions = {
	async save(values: Json, ctx: ActionContext): Promise<ActionEffect> {
		const { name, keyword, text } = values as { name: string; keyword: string; text: string };
		const replaces = editing(ctx);
		const problems = await library.save(
			{ keyword: keyword.trim(), name: name.trim() === '' ? null : name, text },
			{ replaces },
		);
		const errors: Record<string, string> = {};
		for (const [field, problem] of Object.entries(problems)) {
			if (problem !== undefined) errors[field] = problem;
		}
		if (Object.keys(errors).length > 0) return { type: 'fieldErrors', errors };
		return { type: 'pop', toast: replaces === null ? 'Added the snippet' : 'Saved the snippet' };
	},
	async remove(data: Json): Promise<ActionEffect> {
		const { keyword } = data as { keyword: string };
		const deleted = await library.delete(keyword);
		return {
			type: 'toast',
			text: deleted ? 'Deleted the snippet' : 'That snippet was already deleted',
		};
	},
};
