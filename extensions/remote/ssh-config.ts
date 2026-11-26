import { files } from 'dupin';
import { HOST } from './checks.ts';

/** The most hosts listed from the SSH configuration, Include files counted. */
export const MAX_CONFIG_HOSTS = 256;
const MAX_CONFIG_BYTES = 256 * 1024;
const MAX_INCLUDE_BYTES = 64 * 1024;
const MAX_INCLUDE_FILES = 16;
const MAX_INCLUDE_DEPTH = 3;

/** What `ssh_config` splits a line into: words, with double quotes grouping. */
function words(text: string): string[] {
	const out: string[] = [];
	for (const match of text.matchAll(/"([^"]*)"|(\S+)/g)) out.push(match[1] ?? match[2] ?? '');
	return out;
}

/** The literal host names of `Host` lines and the patterns of `Include` lines in `text`. */
export function parseConfig(text: string): { hosts: string[]; includes: string[] } {
	const hosts: string[] = [];
	const includes: string[] = [];
	for (const raw of text.split(/\r?\n/)) {
		const line = raw.trim();
		if (!line || line.startsWith('#')) continue;
		const match = /^([A-Za-z]+)(?:\s*=\s*|\s+)(.*)$/.exec(line);
		if (!match) continue;
		const key = (match[1] ?? '').toLowerCase();
		const rest = match[2] ?? '';
		if (key === 'host') {
			for (const name of words(rest)) {
				// Patterns and negations name no one machine to connect to.
				if (!/[*?!,]/.test(name) && HOST.test(name)) hosts.push(name);
			}
		} else if (key === 'include') {
			includes.push(...words(rest));
		}
	}
	return { hosts, includes };
}

/** `pattern` as `glob(3)` matches one file name: `*` and `?` never match a leading dot. */
function globMatches(pattern: string, name: string): boolean {
	if (name.startsWith('.') && !pattern.startsWith('.')) return false;
	const source = pattern
		.replace(/[.+^${}()|[\]\\]/g, '\\$&')
		.replace(/\*/g, '.*')
		.replace(/\?/g, '.');
	return new RegExp(`^${source}$`).test(name);
}

export interface ConfigHosts {
	hosts: string[];
	/** Why the configuration, or part of it, couldn't be read. */
	problems: string[];
}

function problemOf(error: unknown, what: string): string {
	const code = (error as { code?: unknown } | null)?.code;
	if (code === 'needs-consent') return `Allow Remote to read ${what} in Settings → Extensions`;
	if (code === 'revoked' || code === 'denied') return `Reading ${what} isn't allowed`;
	return `${what} couldn't be read`;
}

/**
 * The hosts the user's SSH configuration names: the file picked for `ssh-config` and the files its
 * `Include` lines name in the folder picked for `ssh-include`, matched by file name, as deep as
 * three levels and sixteen files.
 */
export async function configHosts(): Promise<ConfigHosts> {
	const found = new Set<string>();
	const problems: string[] = [];
	let main: string;
	try {
		main = await files.readText('ssh-config', '', { limit: MAX_CONFIG_BYTES });
	} catch (error) {
		return { hosts: [], problems: [problemOf(error, 'your SSH config')] };
	}
	let parsed = parseConfig(main);
	parsed.hosts.forEach((host) => found.add(host));
	let pending = parsed.includes;
	if (pending.length > 0) {
		let names: string[] | null = null;
		try {
			names = (await files.list('ssh-include', '', { limit: 1000 }))
				.filter((entry) => entry.kind === 'file')
				.map((entry) => entry.name)
				.sort();
		} catch (error) {
			problems.push(problemOf(error, 'the folder your Include lines name'));
		}
		const read = new Set<string>();
		for (let depth = 0; names && depth < MAX_INCLUDE_DEPTH && pending.length > 0; depth++) {
			const next: string[] = [];
			for (const pattern of pending) {
				const base = pattern.split('/').pop() ?? '';
				for (const name of names) {
					if (read.size >= MAX_INCLUDE_FILES || found.size >= MAX_CONFIG_HOSTS) break;
					if (read.has(name) || !globMatches(base, name)) continue;
					read.add(name);
					try {
						parsed = parseConfig(
							await files.readText('ssh-include', name, { limit: MAX_INCLUDE_BYTES }),
						);
					} catch (error) {
						problems.push(problemOf(error, name));
						continue;
					}
					parsed.hosts.forEach((host) => found.add(host));
					next.push(...parsed.includes);
				}
			}
			pending = next;
		}
	}
	return { hosts: [...found].slice(0, MAX_CONFIG_HOSTS), problems };
}
