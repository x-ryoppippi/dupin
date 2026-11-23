import type { Item, SearchContext } from 'dupin';

export function search(query: string, _ctx: SearchContext): Item[] {
	const name = query.trim();
	if (!name) return [];

	const greeting = `Hello, ${name}!`;
	return [{ title: greeting, subtitle: 'Enter copies it', text: greeting }];
}
