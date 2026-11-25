// @ts-check
/// <reference path="../../../sdk/dupin.d.ts" />
import { storage } from 'dupin';

/** @type {{ value: string; title: string }[]} */
const LABELS = [
	{ value: 'none', title: 'None' },
	{ value: 'home', title: 'Home' },
	{ value: 'work', title: 'Work' },
	{ value: 'errands', title: 'Errands' },
];

/**
 * @typedef {{ key: string; title: string; body: string; pinned: boolean; label: string }} Note
 * @typedef {{ id: string; createdAt: string; data: Note }} Stored
 */

/** @returns {Promise<Stored[]>} */
async function notes() {
	const { records } = await storage.list({ limit: 200 });
	return /** @type {Stored[]} */ (/** @type {unknown} */ (records));
}

/**
 * Every note that holds each word typed, pinned ones first, then a row that starts a new one.
 *
 * @param {string} query
 * @returns {Promise<import('dupin').SearchResult>}
 */
export async function search(query) {
	const typed = query.trim();
	const words = typed.toLowerCase().split(/\s+/u).filter(Boolean);
	const found = (await notes())
		.filter(({ data }) =>
			words.every((word) => `${data.title}\n${data.body}`.toLowerCase().includes(word)),
		)
		.sort((a, b) => Number(b.data.pinned) - Number(a.data.pinned));
	/** @type {import('dupin').Section[]} */
	const sections = [];
	if (found.length > 0) sections.push({ title: 'Notes', items: found.map(row) });
	sections.push({
		title: 'New',
		items: [
			{
				id: 'new',
				title: typed ? `New note “${typed}”` : 'New note',
				actions: [{ title: 'Write', push: { view: 'edit', state: { title: typed } } }],
			},
		],
	});
	return { sections };
}

/**
 * @param {Stored} note
 * @returns {import('dupin').Item}
 */
function row({ id, data }) {
	/** @type {import('dupin').Accessory[]} */
	const accessories = [];
	if (data.pinned) accessories.push({ tag: 'Pinned' });
	const label = LABELS.find((option) => option.value === data.label);
	if (label && label.value !== 'none') accessories.push({ text: label.title });
	return {
		id,
		title: data.title,
		subtitle: data.body.split('\n')[0] ?? '',
		accessories,
		actions: [
			{ title: 'Read', push: { view: 'note', state: { id } } },
			{ title: 'Edit', push: { view: 'edit', state: { id } } },
		],
	};
}

/**
 * @param {import('dupin').Json} state
 * @returns {string | undefined}
 */
function idOf(state) {
	return state && typeof state === 'object' && !Array.isArray(state) && typeof state.id === 'string'
		? state.id
		: undefined;
}

/** @type {import('dupin').Views} */
export const views = {
	async note(ctx) {
		const id = idOf(ctx.state);
		const stored = id && (await storage.get(id));
		if (!stored) return { type: 'list', items: [], emptyText: 'This note was deleted.' };
		const note = /** @type {Note} */ (/** @type {unknown} */ (stored.data));
		return {
			type: 'detail',
			title: note.title,
			markdown: note.body,
			metadata: [
				{ label: 'Created', text: stored.createdAt.slice(0, 10) },
				{ label: 'Pinned', text: note.pinned ? 'Yes' : 'No' },
			],
			actions: [
				{ title: 'Edit', push: { view: 'edit', state: { id: stored.id } } },
				...(note.body ? [{ title: 'Copy Text', copy: note.body }] : []),
				{ title: 'Delete', run: 'remove', data: { id: stored.id }, confirm: 'Delete this note?' },
			],
		};
	},

	async edit(ctx) {
		const id = idOf(ctx.state);
		const stored = id ? await storage.get(id) : undefined;
		const state =
			ctx.state && typeof ctx.state === 'object' && !Array.isArray(ctx.state) ? ctx.state : {};
		const note = stored
			? /** @type {Note} */ (/** @type {unknown} */ (stored.data))
			: {
					title: typeof state.title === 'string' ? state.title : '',
					body: '',
					pinned: false,
					label: 'none',
				};
		return {
			type: 'form',
			title: stored ? 'Edit Note' : 'New Note',
			submit: 'save',
			submitTitle: 'Save',
			fields: [
				{
					type: 'text',
					id: 'title',
					title: 'Title',
					required: true,
					default: note.title,
					placeholder: 'What is it about?',
				},
				{ type: 'textarea', id: 'body', title: 'Note', default: note.body },
				{ type: 'checkbox', id: 'pinned', title: 'Pinned', default: note.pinned },
				{ type: 'select', id: 'label', title: 'Label', options: LABELS, default: note.label },
			],
		};
	},
};

/** @type {import('dupin').Actions} */
export const actions = {
	/** A form's values, checked against the other notes before anything is stored. */
	async save(values, ctx) {
		const { title, body, pinned, label } = /** @type {Omit<Note, 'key'>} */ (
			/** @type {unknown} */ (values)
		);
		const all = await notes();
		const id = idOf(ctx.state);
		const editing = all.find((note) => note.id === id);
		const named = title.trim();
		if (
			all.some((note) => note.id !== id && note.data.title.toLowerCase() === named.toLowerCase())
		) {
			return { type: 'fieldErrors', errors: { title: 'Another note has this title' } };
		}
		// A note keeps its key, and so its ID, when it is edited.
		let key = editing?.data.key ?? `note-${Date.now().toString(36)}-${all.length}`;
		while (!editing && all.some((note) => note.data.key === key)) key += '+';
		await storage.put({ key, title: named, body, pinned, label }, { key });
		return { type: 'pop', toast: editing ? 'Saved' : 'Note added' };
	},

	async remove(data) {
		const id = idOf(data);
		if (id) await storage.delete(id);
		return { type: 'pop', toast: 'Deleted' };
	},
};
