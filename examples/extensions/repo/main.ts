import { DupinError, exec, files, storage } from 'dupin';
import type {
	Accessory,
	Action,
	ActionContext,
	ActionEffect,
	CatalogueItem,
	ItemsContext,
	ItemsResult,
	Metadata,
} from 'dupin';

interface Preferences {
	editor: keyof typeof EDITORS;
	terminal: keyof typeof TERMINALS;
	ghq: boolean;
	scanGhq: boolean;
	scanSrc: boolean;
	scanDeveloper: boolean;
	scanProjects: boolean;
	depth: number;
	checks: number;
}

/** What the last `git` reads found of a repository, and when. */
interface Status {
	/** When Dupin last read it, in milliseconds since the epoch. */
	at: number;
	/** The branch, or `null` when HEAD is detached. */
	branch: string | null;
	/** Changed, staged and untracked files. */
	changes: number;
	ahead: number;
	behind: number;
	/** The last commit's time, in milliseconds since the epoch. */
	commit?: number;
	/** `origin`'s URL, without credentials. */
	remote?: string;
}

/** What `items` keeps between refreshes: every repository it listed, by path, and what it read of each. */
interface State {
	repos: string[];
	status: Record<string, Status>;
}

const EDITORS = {
	'Visual Studio Code': 'com.microsoft.VSCode',
	Cursor: 'com.todesktop.230313mzl4w4u92',
	Zed: 'dev.zed.Zed',
	'Sublime Text': 'com.sublimetext.4',
	Nova: 'com.panic.Nova',
	Xcode: 'com.apple.dt.Xcode',
	Finder: 'com.apple.finder',
};

const TERMINALS = {
	Terminal: 'com.apple.Terminal',
	Ghostty: 'com.mitchellh.ghostty',
	iTerm: 'com.googlecode.iterm2',
	WezTerm: 'com.github.wez.wezterm',
};

/** Each files scope, the folder it declares and the preference that turns it on. */
const ROOTS = [
	{ scope: 'ghq', preference: 'scanGhq' },
	{ scope: 'src', preference: 'scanSrc' },
	{ scope: 'developer', preference: 'scanDeveloper' },
	{ scope: 'projects', preference: 'scanProjects' },
] as const;

const STATE_KEY = 'state';
const TTL_SECONDS = 300;
const PAGE = 500;
/**
 * Host requests one refresh makes, of the 256 a call may: three `git` reads for each
 * repository it checks, ghq, storage and each folder's root, and folder listings with the rest.
 */
const MAX_REQUESTS = 240;
const GIT_READS = 3;
/** `git` reads run at once; Dupin runs four extension commands at a time. */
const PARALLEL = 4;
/** No new `git` read starts after this much of the 2 seconds a refresh has. */
const CHECK_BUDGET_MS = 1200;
const SKIPPED = new Set(['node_modules', 'Library', 'vendor']);

export async function items(ctx: ItemsContext): Promise<ItemsResult> {
	const preferences = ctx.preferences as unknown as Preferences;
	if (ctx.reason === 'more') {
		const state = await load();
		return page(state, Number(ctx.cursor ?? 0));
	}
	const started = Date.now();
	const known = await load();
	const checks = Math.max(0, Math.min(32, Math.floor(preferences.checks)));
	const repos = await discover(preferences, MAX_REQUESTS - 8 - GIT_READS * checks);
	const status: Record<string, Status> = {};
	for (const path of repos) {
		const kept = known.status[path];
		if (kept) status[path] = kept;
	}
	await check(repos, status, checks, started);
	const state = { repos: order(repos, status), status };
	await save(state);
	return page(state, 0);
}

/** One part of the catalogue, from `offset`, asking for the next while repositories remain. */
function page(state: State, offset: number): ItemsResult {
	const rows = state.repos
		.slice(offset, offset + PAGE)
		.map((path) => row(path, state.status[path]));
	const next = offset + PAGE;
	const rest = next < state.repos.length ? { more: true, cursor: String(next) } : {};
	// Each part after the first adds to the ones before it.
	if (offset > 0) return { type: 'update', append: rows, ttl: TTL_SECONDS, ...rest };
	return { items: rows, ttl: TTL_SECONDS, ...rest };
}

/** Every repository ghq lists and the folders hold, each once, by absolute path. */
async function discover(preferences: Preferences, allowance: number): Promise<string[]> {
	const found = new Set<string>();
	if (preferences.ghq) {
		for (const path of await ghq()) found.add(path);
	}
	let listings = 0;
	for (const root of ROOTS) {
		if (!preferences[root.preference]) continue;
		const scanned = await scan(root.scope, preferences.depth, allowance - listings);
		listings += scanned.listings;
		for (const path of scanned.repos) found.add(path);
	}
	return [...found];
}

/** The repositories `ghq list -p` prints, or none when ghq isn't allowed or fails. */
async function ghq(): Promise<string[]> {
	const result = await quietly(() => exec.run('ghq-list'));
	if (!result || result.code !== 0 || !result.stdout) return [];
	const lines = result.stdout.split('\n');
	// A cut-off last line names a path that may not be the one ghq meant.
	if (result.truncated) lines.pop();
	return lines.map((line) => line.trim()).filter(isAbsolute);
}

/**
 * The repositories under a files scope, `depth` folders down at most: a folder holding `.git`
 * is one, and nothing inside it is looked at.
 */
async function scan(
	scope: string,
	depth: number,
	allowance: number,
): Promise<{ repos: string[]; listings: number }> {
	const repos: string[] = [];
	if (allowance <= 0) return { repos, listings: 0 };
	const root = await quietly(() => files.root(scope));
	if (!root || !isAbsolute(root)) return { repos, listings: 0 };
	let listings = 0;
	let level = [''];
	for (let current = 0; current <= depth && level.length > 0; current += 1) {
		const next: string[] = [];
		for (const relative of level) {
			if (listings >= allowance) return { repos, listings };
			listings += 1;
			const entries = await quietly(() => files.list(scope, relative, { limit: 1000 }));
			if (!entries) continue;
			if (relative && entries.some((entry) => entry.name === '.git')) {
				repos.push(`${root}/${relative}`);
				continue;
			}
			if (current === depth) continue;
			for (const entry of entries) {
				if (entry.kind !== 'directory' || entry.name.startsWith('.') || SKIPPED.has(entry.name)) {
					continue;
				}
				next.push(relative ? `${relative}/${entry.name}` : entry.name);
			}
		}
		level = next;
	}
	return { repos, listings };
}

/**
 * Reads the Git status of up to `count` repositories, those never read first and then those
 * read longest ago, until the refresh has used its share of time.
 */
async function check(
	repos: string[],
	status: Record<string, Status>,
	count: number,
	started: number,
): Promise<void> {
	const due = [...repos]
		.sort((a, b) => (status[a]?.at ?? 0) - (status[b]?.at ?? 0))
		.slice(0, Math.max(0, count));
	for (let index = 0; index < due.length; index += PARALLEL) {
		if (Date.now() - started > CHECK_BUDGET_MS) return;
		const batch = due.slice(index, index + PARALLEL);
		const read = await Promise.all(batch.map((path) => readStatus(path, status[path])));
		batch.forEach((path, at) => {
			const fresh = read[at];
			if (fresh) status[path] = fresh;
		});
	}
}

async function readStatus(path: string, previous: Status | undefined): Promise<Status | undefined> {
	const result = await quietly(() => exec.run('git-status', { path }));
	if (!result || result.code !== 0 || result.stdout === undefined) return undefined;
	const status: Status = { ...parseStatus(result.stdout), at: Date.now() };
	const commit = await quietly(() => exec.run('git-last-commit', { path }));
	const seconds = Number(commit?.code === 0 ? commit.stdout?.trim() : NaN);
	if (Number.isFinite(seconds) && seconds > 0) status.commit = seconds * 1000;
	else if (previous?.commit) status.commit = previous.commit;
	const remote = await quietly(() => exec.run('git-remote', { path }));
	const url = remote?.code === 0 ? withoutCredentials(remote.stdout?.trim() ?? '') : undefined;
	if (url) status.remote = url;
	return status;
}

/** What `git status --porcelain=v2 --branch` says of the branch and the working tree. */
function parseStatus(output: string): Omit<Status, 'at'> {
	const parsed: Omit<Status, 'at'> = { branch: null, changes: 0, ahead: 0, behind: 0 };
	for (const line of output.split('\n')) {
		if (!line) continue;
		if (line.startsWith('# branch.head ')) {
			const head = line.slice('# branch.head '.length);
			parsed.branch = head === '(detached)' ? null : head;
		} else if (line.startsWith('# branch.ab ')) {
			const [ahead, behind] = line.slice('# branch.ab '.length).split(' ');
			parsed.ahead = Math.abs(Number(ahead)) || 0;
			parsed.behind = Math.abs(Number(behind)) || 0;
		} else if (!line.startsWith('#')) {
			parsed.changes += 1;
		}
	}
	return parsed;
}

/** Repositories with a known last commit, newest first, then the rest by name. */
function order(repos: string[], status: Record<string, Status>): string[] {
	return [...repos].sort((a, b) => {
		const byCommit = (status[b]?.commit ?? 0) - (status[a]?.commit ?? 0);
		if (byCommit !== 0) return byCommit;
		const byName = basename(a).localeCompare(basename(b));
		return byName !== 0 ? byName : a.localeCompare(b);
	});
}

/** Where a repository is from: `owner` and `host` from a ghq-style path or its remote. */
function origin(path: string, remote: string | undefined): { owner: string; host: string } {
	const parts = path.split('/').filter(Boolean);
	const host = parts.at(-3);
	if (host && /^[a-z0-9-]+(\.[a-z0-9-]+)+$/i.test(host)) {
		return { owner: parts.at(-2) ?? '', host };
	}
	const web = remote ? webPage(remote) : undefined;
	if (web) {
		const url = web.slice('https://'.length).split('/');
		return { owner: url.at(-2) ?? '', host: url[0] ?? '' };
	}
	return { owner: parts.at(-2) ?? '', host: '' };
}

function row(path: string, status: Status | undefined): CatalogueItem {
	const name = basename(path);
	const { owner, host } = origin(path, status?.remote);
	const web = status?.remote ? webPage(status.remote) : undefined;
	const data = { path };
	const actions: Action[] = [
		{ title: 'Open in Editor', run: 'openEditor', data },
		{ title: 'Open in Terminal', run: 'openTerminal', data, alternate: true },
		{ title: 'Reveal in Finder', reveal: path },
	];
	if (web) actions.push({ title: `Open on ${siteName(web)}`, open: web });
	actions.push({ title: 'Copy Path', copy: path });
	if (status?.remote) actions.push({ title: 'Copy Remote URL', copy: status.remote });
	// The folders above a repository may include the home folder, whose name would find every one.
	const keywords = [...new Set([owner && `${owner}/${name}`, owner, host])]
		.filter((word): word is string => Boolean(word) && bytes(word) <= 64 && word !== name)
		.slice(0, 16);
	const item: CatalogueItem = {
		id: `r${hash(path)}`,
		title: fit(name, 512),
		subtitle: fit([owner, host].filter(Boolean).join(' · ') || parent(path), 2048),
		keywords,
		actions,
		detail: { metadata: metadata(path, status) },
	};
	if (status?.commit) item.time = status.commit;
	const accessories = tags(status);
	if (accessories.length > 0) item.accessories = accessories;
	return item;
}

function tags(status: Status | undefined): Accessory[] {
	if (!status) return [];
	const tags: Accessory[] = [];
	const drift = [status.ahead && `↑${status.ahead}`, status.behind && `↓${status.behind}`]
		.filter(Boolean)
		.join(' ');
	tags.push({ tag: fit([status.branch ?? 'detached', drift].filter(Boolean).join(' '), 64) });
	if (status.changes > 0)
		tags.push({ tag: status.changes === 1 ? '1 change' : `${status.changes} changes` });
	tags.push({ freshness: status.at });
	return tags;
}

function metadata(path: string, status: Status | undefined): Metadata[] {
	const entries: Metadata[] = [{ label: 'Path', text: fit(path, 512) }];
	if (!status) {
		entries.push({ label: 'Status', text: 'Git hasn’t been read for it yet' });
		return entries;
	}
	entries.push({ label: 'Branch', text: fit(status.branch ?? 'Detached HEAD', 512) });
	entries.push({
		label: 'Working tree',
		text:
			status.changes === 0
				? 'Clean'
				: status.changes === 1
					? '1 changed file'
					: `${status.changes} changed files`,
	});
	if (status.ahead || status.behind) {
		entries.push({ label: 'Upstream', text: `${status.ahead} ahead, ${status.behind} behind` });
	}
	if (status.commit) entries.push({ label: 'Last commit', text: date(status.commit) });
	if (status.remote) entries.push({ label: 'Remote', text: fit(status.remote, 512) });
	entries.push({ label: 'Checked', text: date(status.at) });
	return entries;
}

/**
 * `remote` without a user name or password where one could hide a token: an `http(s)` URL
 * loses its user information and an `ssh` URL its password. Anything else, such as a local
 * path, is left out.
 */
function withoutCredentials(remote: string): string | undefined {
	if (!remote || /[\s\u0000-\u001f\u007f]/u.test(remote)) return undefined;
	const url = /^(https?|ssh|git):\/\/(?:([^@/]*)@)?([^/]+)(\/.*)?$/iu.exec(remote);
	if (url) {
		const [, scheme, user, host, rest] = url;
		const keptUser = scheme.toLowerCase() === 'ssh' && user ? `${user.split(':')[0]}@` : '';
		return `${scheme}://${keptUser}${host}${rest ?? ''}`;
	}
	if (/^[a-z0-9._-]+@[a-z0-9.-]+:[^/].*$/iu.test(remote)) return remote;
	return undefined;
}

/**
 * The HTTPS page of a remote on a host, such as `https://github.com/owner/name` for
 * `git@github.com:owner/name.git`, or `undefined` for one that isn't on a host.
 */
function webPage(remote: string): string | undefined {
	let host: string;
	let path: string;
	const url = /^(?:https?|ssh|git):\/\/(?:[^@/]*@)?([^/:]+)(?::\d+)?\/(.+)$/iu.exec(remote);
	const scp = /^[^@/]+@([^:/]+):(.+)$/u.exec(remote);
	if (url) [, host, path] = url;
	else if (scp) [, host, path] = scp;
	else return undefined;
	host = host.toLowerCase();
	path = path.replace(/\.git\/?$/u, '').replace(/\/+$/u, '');
	if (!/^[a-z0-9-]+(\.[a-z0-9-]+)+$/u.test(host)) return undefined;
	const parts = path.split('/');
	if (
		parts.length < 2 ||
		parts.some((part) => !/^[A-Za-z0-9._~-]+$/u.test(part) || part === '.' || part === '..')
	) {
		return undefined;
	}
	return `https://${host}/${parts.join('/')}`;
}

function siteName(web: string): string {
	const host = web.slice('https://'.length).split('/')[0];
	if (host === 'github.com') return 'GitHub';
	if (host === 'gitlab.com') return 'GitLab';
	if (host === 'bitbucket.org') return 'Bitbucket';
	if (host === 'codeberg.org') return 'Codeberg';
	return host;
}

export const actions = {
	async openEditor(data: unknown, ctx: ActionContext): Promise<ActionEffect> {
		const preferences = ctx.preferences as unknown as Preferences;
		const title = preferences.editor in EDITORS ? preferences.editor : 'Visual Studio Code';
		return openIn(data, EDITORS[title], title);
	},
	async openTerminal(data: unknown, ctx: ActionContext): Promise<ActionEffect> {
		const preferences = ctx.preferences as unknown as Preferences;
		const title = preferences.terminal in TERMINALS ? preferences.terminal : 'Terminal';
		return openIn(data, TERMINALS[title], title);
	},
};

/** Opens the repository in the app `bundle`, once it is one the last refresh listed. */
async function openIn(data: unknown, bundle: string, title: string): Promise<ActionEffect> {
	const path =
		typeof data === 'object' && data !== null ? (data as { path?: unknown }).path : undefined;
	if (typeof path !== 'string' || !isAbsolute(path)) {
		return { type: 'toast', text: 'That isn’t a repository Dupin listed' };
	}
	const state = await load();
	if (!state.repos.includes(path)) {
		return { type: 'toast', text: 'That repository is no longer listed; type repo again' };
	}
	const result = await exec.run('open-app', { app: bundle, path });
	if (result.code === 0) return { type: 'hide' };
	return {
		type: 'toast',
		text: `Couldn’t open it in ${title}; choose another in Settings → Extensions → Repositories`,
	};
}

async function load(): Promise<State> {
	const record = await quietly(async () => {
		const { records } = await storage.list({ limit: 1 });
		return records[0];
	});
	const data = record?.data as unknown as State | undefined;
	if (
		!data ||
		!Array.isArray(data.repos) ||
		typeof data.status !== 'object' ||
		data.status === null
	) {
		return { repos: [], status: {} };
	}
	return data;
}

async function save(state: State): Promise<void> {
	await quietly(() =>
		storage.put(state as unknown as Parameters<typeof storage.put>[0], { key: STATE_KEY }),
	);
}

/**
 * `run`'s result, or `undefined` when Dupin refused or the request failed: a scope the user
 * hasn't allowed or a folder they don't have leaves that source out, and Dupin shows any
 * question about it under the keyword.
 */
async function quietly<T>(run: () => Promise<T>): Promise<T | undefined> {
	try {
		return await run();
	} catch (error) {
		if (error instanceof DupinError) return undefined;
		throw error;
	}
}

function isAbsolute(path: string): boolean {
	return (
		path.startsWith('/') &&
		!/[\u0000-\u001f\u007f]/u.test(path) &&
		!path.split('/').some((part) => part === '.' || part === '..')
	);
}

function basename(path: string): string {
	return path.split('/').filter(Boolean).at(-1) ?? path;
}

function parent(path: string): string {
	return path.split('/').filter(Boolean).at(-2) ?? '';
}

/** `text` as UTF-8, a lone surrogate as U+FFFD, as it reaches Dupin. */
function utf8(text: string): number[] {
	const out: number[] = [];
	for (const char of text) {
		let code = char.codePointAt(0) ?? 0xfffd;
		if (code >= 0xd800 && code <= 0xdfff) code = 0xfffd;
		if (code < 0x80) out.push(code);
		else if (code < 0x800) out.push(0xc0 | (code >> 6), 0x80 | (code & 0x3f));
		else if (code < 0x10000)
			out.push(0xe0 | (code >> 12), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f));
		else {
			out.push(
				0xf0 | (code >> 18),
				0x80 | ((code >> 12) & 0x3f),
				0x80 | ((code >> 6) & 0x3f),
				0x80 | (code & 0x3f),
			);
		}
	}
	return out;
}

function bytes(text: string): number {
	return utf8(text).length;
}

/** `text` cut to `limit` UTF-8 bytes, ending with `…` when cut. */
function fit(text: string, limit: number): string {
	if (bytes(text) <= limit) return text;
	let cut = text;
	while (bytes(`${cut}…`) > limit) cut = cut.slice(0, -1);
	return `${cut}…`;
}

/** Two FNV-1a hashes as hex: an ID that stays with the path and fits the 128 bytes an ID may have. */
function hash(text: string): string {
	const fnv = (seed: number) => {
		let value = seed;
		for (const byte of utf8(text)) value = Math.imul(value ^ byte, 0x01000193);
		return (value >>> 0).toString(16).padStart(8, '0');
	};
	return fnv(0x811c9dc5) + fnv(0x050c5d1f);
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

function two(value: number): string {
	return String(value).padStart(2, '0');
}

/** `25 Nov 2026, 14:05`. */
function date(ms: number): string {
	const when = new Date(ms);
	return `${when.getDate()} ${MONTHS[when.getMonth()]} ${when.getFullYear()}, ${two(when.getHours())}:${two(when.getMinutes())}`;
}
