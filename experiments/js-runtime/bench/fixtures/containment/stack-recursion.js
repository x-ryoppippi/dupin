function deeper(depth) {
	return deeper(depth + 1) + 1;
}

export function search() {
	return deeper(0);
}
