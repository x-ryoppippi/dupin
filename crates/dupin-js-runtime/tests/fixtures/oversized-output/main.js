export function search() {
	const text = 'x'.repeat(16 * 1024);
	return Array.from({ length: 50 }, (_, index) => ({ title: `Row ${index}`, text }));
}
