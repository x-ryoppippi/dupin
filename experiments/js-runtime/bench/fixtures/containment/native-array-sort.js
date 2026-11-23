export function search() {
	const values = Array.from({ length: 300_000 }, (_, index) => (index * 7919) % 300_000);
	values.sort();
	return [{ title: String(values[0]) }];
}
