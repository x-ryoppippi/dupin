import { storage } from 'dupin';

export async function search(query) {
	const text = query.trim();
	let saved = [];
	try {
		saved = (await storage.list({ limit: 20 })).records;
	} catch (error) {
		return [{ title: `Notes can't read its storage`, subtitle: error.message }];
	}
	const rows = saved.map((note) => ({ title: note.data, text: note.data }));
	if (text)
		rows.unshift({
			title: `Save “${text}”`,
			actions: [{ title: 'Save', run: 'save', data: { text } }],
		});
	return rows;
}

export const actions = {
	async save({ text }) {
		await storage.put(text);
		return { type: 'toast', text: 'Saved' };
	},
};
