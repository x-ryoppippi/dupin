import { storage } from 'dupin';
import type { Json } from 'dupin';
import {
	DEFAULT_LINES,
	MAX_LINES,
	MAX_SOURCES,
	MIN_LINES,
	PATH,
	UNIT,
	validHost,
	validTerminal,
} from './checks.ts';
import type { Terminal } from './checks.ts';
import type { Row } from './parse.ts';

/** The most hosts the package keeps settings for. */
export const MAX_HOSTS = 64;
const MAX_RECORDS = 2000;

export interface Settings {
	terminal: Terminal;
	/** Hosts the user named beside those in the SSH configuration. */
	hosts: string[];
}

export interface HostSettings {
	host: string;
	enabled: boolean;
	units: string[];
	logs: string[];
	dirs: string[];
	lines: number;
	/** Whether Terminate is offered for this host's processes. */
	allowTerminate: boolean;
}

/** How the last run of a probe ended. */
export type State = 'ok' | 'failed' | 'unreachable' | 'auth' | 'host-key' | 'timeout' | 'consent';

/** What one probe of a host found, as last fetched. */
export interface Probed {
	host: string;
	probe: string;
	fetchedAt: number;
	state: State;
	message?: string;
	truncated?: boolean;
	rows: Row[];
}

export interface Stored {
	settings: Settings;
	hosts: Map<string, HostSettings>;
	/** By host, then probe id: the record's storage id and what it holds. */
	probes: Map<string, Map<string, { id: string; probed: Probed }>>;
}

const STATES: readonly State[] = [
	'ok',
	'failed',
	'unreachable',
	'auth',
	'host-key',
	'timeout',
	'consent',
];

export function defaultHost(host: string): HostSettings {
	return {
		host,
		enabled: false,
		units: [],
		logs: [],
		dirs: [],
		lines: DEFAULT_LINES,
		allowTerminate: false,
	};
}

function strings(value: unknown, pattern: RegExp): string[] {
	return Array.isArray(value)
		? value
				.filter((item): item is string => typeof item === 'string' && pattern.test(item))
				.slice(0, MAX_SOURCES)
		: [];
}

function readSettings(data: Record<string, unknown>): Settings {
	const hosts = Array.isArray(data.hosts) ? data.hosts.map(validHost) : [];
	return {
		terminal: validTerminal(data.terminal),
		hosts: hosts.filter((host): host is string => host !== null).slice(0, MAX_HOSTS),
	};
}

function readHost(data: Record<string, unknown>): HostSettings | null {
	const host = validHost(data.host);
	if (host === null) return null;
	const lines =
		typeof data.lines === 'number' && Number.isInteger(data.lines) ? data.lines : DEFAULT_LINES;
	return {
		host,
		enabled: data.enabled === true,
		units: strings(data.units, UNIT),
		logs: strings(data.logs, PATH),
		dirs: strings(data.dirs, PATH),
		lines: Math.min(MAX_LINES, Math.max(MIN_LINES, lines)),
		allowTerminate: data.allowTerminate === true,
	};
}

function readProbed(data: Record<string, unknown>): Probed | null {
	const host = validHost(data.host);
	if (host === null || typeof data.probe !== 'string' || typeof data.fetchedAt !== 'number')
		return null;
	const state = STATES.find((known) => known === data.state);
	if (state === undefined || !Array.isArray(data.rows)) return null;
	return {
		host,
		probe: data.probe,
		fetchedAt: data.fetchedAt,
		state,
		message: typeof data.message === 'string' ? data.message : undefined,
		truncated: data.truncated === true,
		rows: data.rows as Row[],
	};
}

/** Everything the package keeps, read in one pass. */
export async function load(): Promise<Stored> {
	const stored: Stored = {
		settings: { terminal: 'Terminal', hosts: [] },
		hosts: new Map(),
		probes: new Map(),
	};
	let before: string | undefined;
	let seen = 0;
	let settingsSeen = false;
	do {
		const page = await storage.list({ before, limit: 200 });
		for (const record of page.records) {
			const data = record.data;
			if (data === null || typeof data !== 'object' || Array.isArray(data)) continue;
			const fields = data as Record<string, unknown>;
			// Pages run newest first, so the first record of each kind or key is the one that counts.
			if (fields.kind === 'settings') {
				if (!settingsSeen) stored.settings = readSettings(fields);
				settingsSeen = true;
			} else if (fields.kind === 'host') {
				const host = readHost(fields);
				if (host !== null && !stored.hosts.has(host.host) && stored.hosts.size < MAX_HOSTS) {
					stored.hosts.set(host.host, host);
				}
			} else if (fields.kind === 'probe') {
				const probed = readProbed(fields);
				if (probed === null) continue;
				const probes = stored.probes.get(probed.host) ?? new Map();
				if (!probes.has(probed.probe)) probes.set(probed.probe, { id: record.id, probed });
				stored.probes.set(probed.host, probes);
			}
		}
		seen += page.records.length;
		before = page.next;
	} while (before !== undefined && seen < MAX_RECORDS);
	return stored;
}

export async function saveSettings(settings: Settings): Promise<void> {
	await storage.put({ kind: 'settings', ...settings } as unknown as Json, { key: 'settings' });
}

export async function saveHost(host: HostSettings): Promise<void> {
	await storage.put({ kind: 'host', ...host } as unknown as Json, { key: `host:${host.host}` });
}

export async function saveProbed(probed: Probed): Promise<void> {
	await storage.put({ kind: 'probe', ...probed } as unknown as Json, {
		key: `probe:${probed.host}:${probed.probe}`,
	});
}

/** Deletes what was fetched for `host`, or for every host, except the probes in `keep`. */
export async function forget(stored: Stored, host: string | null, keep: Set<string> = new Set()) {
	for (const [name, probes] of stored.probes) {
		if (host !== null && name !== host) continue;
		for (const [probe, { id }] of probes) {
			if (!keep.has(probe)) await storage.delete(id);
		}
	}
}
