// The bench prepends `const DATA = [...]` with 400 generated candidates.
const CANDIDATES = DATA.map((candidate) => ({
	...candidate,
	haystack: `${candidate.title} ${candidate.subtitle}`.toLowerCase(),
}));

function score(text, query) {
	let position = 0;
	let previous = -2;
	let total = 0;
	for (const char of query) {
		const found = text.indexOf(char, position);
		if (found < 0) return -1;
		total += found === previous + 1 ? 3 : 1;
		if (found === 0 || text[found - 1] === ' ' || text[found - 1] === '/') total += 2;
		previous = found;
		position = found + 1;
	}
	return total - text.length / 100;
}

export function search(query) {
	const needle = query.trim().toLowerCase();
	const matches = [];
	for (const candidate of CANDIDATES) {
		const value = score(candidate.haystack, needle);
		if (value >= 0) matches.push({ candidate, value });
	}
	matches.sort((a, b) => b.value - a.value || (a.candidate.title < b.candidate.title ? -1 : 1));
	return matches.slice(0, 50).map(({ candidate }) => ({
		id: candidate.id,
		title: candidate.title,
		subtitle: candidate.subtitle,
		text: candidate.path,
	}));
}
