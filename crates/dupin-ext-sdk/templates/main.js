/// <reference path="./dupin.d.ts" />

/**
 * @param {string} query
 * @returns {import('dupin').Item[]}
 */
export function search(query) {
	const name = query.trim();
	if (!name) return [];

	const greeting = `Hello, ${name}!`;
	return [{ title: greeting, subtitle: 'Enter copies it', text: greeting }];
}
