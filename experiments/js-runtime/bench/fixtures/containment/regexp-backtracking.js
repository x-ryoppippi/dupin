export function search() {
	return [{ title: String(/^(a+)+$/.test('a'.repeat(64) + '!')) }];
}
