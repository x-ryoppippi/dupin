function spin() {
	return Promise.resolve().then(spin);
}

export function search() {
	spin();
	return new Promise(() => {});
}
