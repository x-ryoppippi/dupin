export function search() {
	return [];
}

export const filters = {
	longest(input) {
		return input.candidates
			.map((candidate, index) => [candidate.title.length, index])
			.sort((a, b) => b[0] - a[0] || a[1] - b[1])
			.map(([, index]) => index);
	},
};
