// 600 repositories shaped as extensions/repo lists them: an owner and host, the keywords that
// find them, a branch tag, the actions and the detail. A latency run allows no ghq, Git or
// folders, so this copy, which replaces the bundled package by its name, hands Dupin the rows
// the bundled one would; a keypress only matches the catalogue Dupin holds either way. Every
// row is the same on every run.
const COUNT = 600;
const PART = 500;
const HOUR_MS = 3600 * 1000;
const NOW = Date.UTC(2026, 10, 26, 9);

const owners = [
	'ryoppippi',
	'x-ryoppippi',
	'oven-sh',
	'denoland',
	'rust-lang',
	'vitejs',
	'sveltejs',
	'nixos',
];
const hosts = ['github.com', 'github.com', 'github.com', 'gitlab.com'];
const names = [
	'ccusage',
	'dupin',
	'bun',
	'deno',
	'cargo',
	'vite',
	'svelte',
	'nixpkgs',
	'dotfiles',
	'website',
	'rune',
	'sitemcp',
	'unplugin',
	'typescript',
	'clippy',
];
const branches = ['main', 'main', 'main', 'feat/search', 'fix/latency', 'release'];

function row(index) {
	const owner = owners[index % owners.length];
	const host = hosts[(index * 3) % hosts.length];
	const name = `${names[(index * 7) % names.length]}${index < names.length ? '' : `-${index}`}`;
	const path = `/Users/dupin/ghq/${host}/${owner}/${name}`;
	const remote = `git@${host}:${owner}/${name}.git`;
	const data = { path };
	return {
		id: `r${index}`,
		title: name,
		subtitle: `${owner} · ${host}`,
		keywords: [`${owner}/${name}`, owner, host],
		time: NOW - index * HOUR_MS,
		accessories: [
			{ tag: branches[index % branches.length] },
			...(index % 5 === 0 ? [{ tag: `${index % 7} changes` }] : []),
			{ freshness: NOW - (index % 9) * 60 * 1000 },
		],
		actions: [
			{ title: 'Open in Editor', run: 'openEditor', data },
			{ title: 'Open in Terminal', run: 'openTerminal', data, alternate: true },
			{ title: 'Reveal in Finder', reveal: path },
			{ title: 'Open on GitHub', open: `https://${host}/${owner}/${name}` },
			{ title: 'Copy Path', copy: path },
			{ title: 'Copy Remote URL', copy: remote },
		],
		detail: {
			metadata: [
				{ label: 'Path', text: path },
				{ label: 'Branch', text: branches[index % branches.length] },
				{ label: 'Remote', text: remote },
			],
		},
	};
}

export function items(ctx) {
	const start = ctx.reason === 'more' ? Number(ctx.cursor) : 0;
	const end = Math.min(start + PART, COUNT);
	const rows = Array.from({ length: end - start }, (_, offset) => row(start + offset));
	const rest = end < COUNT ? { more: true, cursor: String(end) } : {};
	if (start > 0) return { type: 'update', append: rows, ...rest };
	return { items: rows, ...rest };
}

export const actions = {
	openEditor() {},
	openTerminal() {},
};
