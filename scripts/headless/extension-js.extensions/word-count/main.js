export function search(query) {
	const input = query.trim();
	if (!input) return [];

	const count = input.split(/\s+/u).length;
	return [
		{ title: `${count} words`, text: String(count) },
		{
			title: 'Count words on the web',
			subtitle: input,
			url: `https://www.google.com/search?q=${encodeURIComponent(`word count ${input}`)}`,
		},
	];
}
