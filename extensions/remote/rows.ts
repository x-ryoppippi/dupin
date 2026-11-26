import type { Action, CatalogueItem, Metadata } from 'dupin';
import { processName } from './parse.ts';
import type { PortRow, ProcessRow } from './parse.ts';
import { probesOf, unanswered } from './probes.ts';
import type { Probe } from './probes.ts';
import type { HostSettings, Probed, State } from './store.ts';
import { clamp, line } from './text.ts';

export const LABEL: Record<State, string> = {
	ok: 'online',
	failed: 'error',
	unreachable: 'unreachable',
	auth: 'auth failed',
	'host-key': 'host key',
	timeout: 'timed out',
	consent: 'needs consent',
};

export type Fetched = Map<string, { id: string; probed: Probed }>;

/** What a row's Show Details finds it by: the record it was read from, while it holds that fetch. */
export interface RowRef {
	host: string;
	record: string;
	index: number;
	fetchedAt: number;
}

/** A short, stable name for a probe in row IDs, which a path could make too long. */
function hash(text: string): string {
	let value = 0x811c9dc5;
	for (let index = 0; index < text.length; index++) {
		value ^= text.charCodeAt(index);
		value = Math.imul(value, 0x01000193) >>> 0;
	}
	return value.toString(16).padStart(8, '0');
}

function meta(label: string, text: string): Metadata {
	return { label, text: line(text, 512) || '—' };
}

function terminal(host: string): Action {
	return { title: 'Open SSH Session', run: 'terminal', data: { host }, alternate: true };
}

function details(ref: RowRef): Action {
	return { title: 'Show Details', push: { view: 'details', state: { ...ref } } };
}

/** The host's row: how it answered last, and how long ago Dupin says that was. */
export function hostItem(config: HostSettings, fetched: Fetched | undefined): CatalogueItem {
	const host = config.host;
	const first = fetched?.get('ps')?.probed;
	const counts: string[] = [];
	for (const probe of probesOf(config)) {
		const probed = fetched?.get(probe.id)?.probed;
		if (probed === undefined) continue;
		const noun =
			probe.kind === 'processes'
				? 'processes'
				: probe.kind === 'ports'
					? 'ports'
					: probe.kind === 'log'
						? 'log lines'
						: 'files';
		counts.push(`${probed.rows.length} ${noun}`);
	}
	const subtitle =
		first === undefined
			? 'Not fetched yet: ⌘↩ fetches it'
			: first.state !== 'ok' && first.message
				? first.message
				: counts.join(' · ');
	const state = first === undefined ? 'not fetched' : LABEL[first.state];
	return {
		id: `host:${host}`,
		title: host,
		subtitle: line(subtitle, 512),
		section: 'Hosts',
		icon: 'sf:server.rack',
		...(first === undefined ? {} : { time: first.fetchedAt }),
		accessories: [{ tag: state }],
		matchText: `${host} host`,
		detail: {
			metadata: [
				meta('Host', host),
				meta('State', state),
				...(first?.message ? [meta('Last error', first.message)] : []),
				meta('Log units', config.units.join(', ')),
				meta('Log files', config.logs.join(', ')),
				meta('Folders', config.dirs.join(', ')),
				meta('Terminate', config.allowTerminate ? 'Allowed' : 'Not allowed'),
			],
		},
		actions: [
			{
				title: `Show ${host}`,
				push: {
					view: 'host',
					state: {
						host,
						records: [...(fetched?.values() ?? [])].map((entry) => entry.id).slice(0, 64),
						terminate: config.allowTerminate,
					},
				},
			},
			{ title: `Refresh ${host}`, run: 'refresh', data: { host }, alternate: true },
			{ title: 'Open SSH Session', run: 'terminal', data: { host } },
			{ title: 'Copy Host Name', copy: host },
			{ title: `Configure ${host}…`, push: { view: 'configure', state: { host } } },
		],
	};
}

function processItem(
	host: string,
	row: ProcessRow,
	terminate: boolean,
	ref: RowRef,
): CatalogueItem {
	const name = line(processName(row), 128);
	const actions: Action[] = [{ title: 'Copy PID', copy: row.pid }, details(ref), terminal(host)];
	if (terminate) {
		actions.push({
			title: 'Terminate',
			run: 'terminate',
			data: { host, pid: row.pid, name },
			confirm: clamp(`Send TERM to ${name} (PID ${row.pid}) on ${host}?`, 256),
		});
	}
	return {
		id: `ps:${host}:${row.pid}`,
		title: clamp(row.command || name, 256),
		subtitle: line(
			`${host} · PID ${row.pid} · ${row.user} · ${row.cpu}% CPU · ${row.mem}% memory · up ${row.elapsed}`,
			512,
		),
		icon: 'sf:gearshape',
		text: row.command || row.pid,
		matchText: clamp(`${host} ps processes ${row.pid} ${row.user} ${row.command}`, 4096),
		detail: {
			metadata: [
				meta('Host', host),
				meta('PID', row.pid),
				meta('User', row.user),
				meta('CPU', `${row.cpu}%`),
				meta('Memory', `${row.mem}%`),
				meta('Running for', row.elapsed),
				meta('Command', row.command),
			],
		},
		actions,
	};
}

function portItem(host: string, row: PortRow, index: number, ref: RowRef): CatalogueItem {
	return {
		id: `port:${host}:${index}`,
		title: line(`${row.proto} ${row.local}`, 256),
		subtitle: line([host, row.state, row.process].filter(Boolean).join(' · '), 512),
		icon: 'sf:network',
		text: row.local || row.proto,
		matchText: clamp(`${host} ports port ${row.proto} ${row.local} ${row.process}`, 4096),
		detail: {
			metadata: [
				meta('Host', host),
				meta('Protocol', row.proto),
				meta('State', row.state),
				meta('Address', row.local),
				meta('Peer', row.peer),
				meta('Process', row.process),
			],
		},
		actions: [details(ref), terminal(host)],
	};
}

function logItem(
	host: string,
	probe: Probe,
	text: string,
	index: number,
	ref: RowRef,
): CatalogueItem {
	return {
		id: `log:${host}:${hash(probe.id)}:${index}`,
		title: clamp(text, 256),
		subtitle: line(`${host} · ${probe.title}`, 512),
		icon: 'sf:text.alignleft',
		text,
		matchText: clamp(`${host} logs log ${probe.title} ${text}`, 4096),
		actions: [details(ref), terminal(host)],
	};
}

function fileItem(
	host: string,
	probe: Probe,
	path: string,
	index: number,
	ref: RowRef,
): CatalogueItem {
	const slash = path.lastIndexOf('/');
	return {
		id: `file:${host}:${hash(probe.id)}:${index}`,
		title: line(path.slice(slash + 1) || path, 256),
		subtitle: line(`${host} · ${path.slice(0, Math.max(slash, 1))}`, 512),
		icon: 'sf:doc',
		text: path,
		matchText: clamp(`${host} files file ${path}`, 4096),
		actions: [
			{ title: 'Copy as host:path', copy: clamp(`${host}:${path}`, 16 * 1024) },
			details(ref),
			terminal(host),
		],
	};
}

/** The rows one probe found, in its section, or one row saying why it found none. */
export function probeItems(
	config: HostSettings,
	probe: Probe,
	entry: { id: string; probed: Probed },
	hostState: State | undefined,
): CatalogueItem[] {
	const host = config.host;
	const { probed } = entry;
	const section = line(`${host} · ${probe.title}`, 128);
	const stale = hostState !== undefined && unanswered(hostState) ? [{ tag: LABEL[hostState] }] : [];
	const ref = (index: number): RowRef => ({
		host,
		record: entry.id,
		index,
		fetchedAt: probed.fetchedAt,
	});
	const rows: CatalogueItem[] = probed.rows.map((row, index) => {
		if (probe.kind === 'processes') {
			return processItem(host, row as ProcessRow, config.allowTerminate, ref(index));
		}
		if (probe.kind === 'ports') return portItem(host, row as PortRow, index, ref(index));
		if (probe.kind === 'log')
			return logItem(host, probe, (row as { line: string }).line, index, ref(index));
		return fileItem(host, probe, (row as { path: string }).path, index, ref(index));
	});
	if (probed.state !== 'ok' && rows.length === 0) {
		rows.push({
			id: `problem:${host}:${hash(probe.id)}`,
			title: line(`${probe.title}: ${probed.message ?? LABEL[probed.state]}`, 256),
			subtitle: line(host, 512),
			icon: 'sf:exclamationmark.triangle',
			matchText: clamp(`${host} ${probe.kind} ${probe.title} ${probed.message ?? ''}`, 4096),
			actions: [{ title: `Refresh ${host}`, run: 'refresh', data: { host } }, terminal(host)],
		});
	}
	return rows.map((row) => ({
		...row,
		section,
		time: probed.fetchedAt,
		...(stale.length > 0 ? { accessories: stale } : {}),
	}));
}

/** Every row the host's fetched probes hold, in the order its settings list them. */
export function hostRows(config: HostSettings, fetched: Fetched | undefined): CatalogueItem[] {
	const state = fetched?.get('ps')?.probed.state;
	const rows: CatalogueItem[] = [];
	for (const probe of probesOf(config)) {
		const entry = fetched?.get(probe.id);
		if (entry) rows.push(...probeItems(config, probe, entry, state));
	}
	return rows;
}
