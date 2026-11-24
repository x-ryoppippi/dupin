// 5,000 rows in parts of 500, the most one answer may carry, so loading the catalogue also
// exercises `more` and `cursor`. Every row is the same on every run.
const ROWS = 5000;
const PART = 500;

const first = [
	'Terminal',
	'Safari',
	'Rust',
	'Notes',
	'Archive',
	'Budget',
	'Calendar',
	'Draft',
	'Invoice',
	'Journal',
];
const second = [
	'settings',
	'history',
	'report',
	'backup',
	'project',
	'review',
	'summary',
	'export',
	'meeting',
	'release',
];
const places = ['Work', 'Home', 'Shared', 'Personal', 'Team'];

function row(index) {
	const a = first[(index * 7) % first.length];
	const b = second[(index * 13) % second.length];
	const place = places[index % places.length];
	return {
		id: `row-${index}`,
		title: `${a} ${b} ${index}`,
		subtitle: `${place} · item ${index} of ${ROWS}`,
		keywords: [place.toLowerCase(), b],
		text: `${a} ${b} ${index}`,
	};
}

export function items(ctx) {
	const start = ctx.reason === 'more' ? Number(ctx.cursor) : 0;
	const end = Math.min(start + PART, ROWS);
	const rows = [];
	for (let index = start; index < end; index += 1) rows.push(row(index));
	const more = end < ROWS;
	return start === 0
		? { items: rows, cursor: String(end), more }
		: { type: 'update', append: rows, cursor: String(end), more };
}
