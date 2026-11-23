function fork() {
	Promise.resolve().then(fork);
	Promise.resolve().then(fork);
}

export function search() {
	fork();
	return new Promise(() => {});
}
