// Docs pages are separate documents, so the sidebar would reopen scrolled to the top on every
// link. This runs inline after the body, before first paint, hence it must not import anything:
// the host inlines it with `toString()`.
export function keepSidebarScroll() {
	const key = 'docs-sidebar-scroll';
	const sidebar = document.querySelector<HTMLElement>('.docs-sidebar');
	if (!sidebar) return;

	// Below 820px the list is a collapsed `<details>` and the page scrolls, not the sidebar.
	if (sidebar.scrollHeight > sidebar.clientHeight) {
		let saved: number | undefined;
		try {
			const value = sessionStorage.getItem(key);
			if (value !== null) saved = Number(value);
		} catch {
			// Storage can be blocked; fall back to the current page's link.
		}
		const current = sidebar.querySelector<HTMLElement>('[aria-current="page"]');
		if (saved !== undefined && Number.isFinite(saved)) {
			sidebar.scrollTop = saved;
		} else if (current) {
			const box = sidebar.getBoundingClientRect();
			const link = current.getBoundingClientRect();
			sidebar.scrollTop += link.top - box.top - (box.height - link.height) / 2;
		}
	}

	const save = () => {
		try {
			sessionStorage.setItem(key, String(sidebar.scrollTop));
		} catch {
			// Nothing to do: the next page scrolls to its own link.
		}
	};
	sidebar.addEventListener('click', save);
	addEventListener('pagehide', save);
}
