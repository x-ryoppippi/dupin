export function search() {
	const haystack = 'a'.repeat(4_000_000);
	const needle = 'a'.repeat(2_000_000) + 'b';
	return [{ title: String(haystack.indexOf(needle)) }];
}
