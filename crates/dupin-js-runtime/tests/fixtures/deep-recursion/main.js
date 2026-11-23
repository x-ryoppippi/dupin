function depth(n) {
	return depth(n + 1) + 1;
}

export function search() {
	return [{ title: String(depth(0)) }];
}
