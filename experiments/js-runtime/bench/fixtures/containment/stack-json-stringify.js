export function search() {
	let value = {};
	for (let depth = 0; depth < 200_000; depth++) value = { value };
	return JSON.stringify(value);
}
