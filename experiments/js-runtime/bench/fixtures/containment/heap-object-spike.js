export function search() {
	const rows = [];
	for (;;) rows.push({ id: String(rows.length), title: `row ${rows.length}` });
}
