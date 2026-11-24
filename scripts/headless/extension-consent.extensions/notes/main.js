import { storage } from 'dupin';

export async function search(query) {
	const text = query.trim();
	let saved = [];
	try {
		saved = (await storage.get('notes')) ?? [];
	} catch (error) {
		return [{ title: `Notes can't read its storage`, subtitle: error.message }];
	}
	const rows = saved.map((note) => ({ title: note, text: note }));
	if (text)
		rows.unshift({
			title: `Save “${text}”`,
			actions: [{ title: 'Save', run: 'save', data: { text } }],
		});
	return rows;
}

export const actions = {
	async save({ text }) {
		const saved = (await storage.get('notes')) ?? [];
		await storage.set('notes', [text, ...saved].slice(0, 20));
		return { type: 'toast', text: 'Saved' };
	},
};
