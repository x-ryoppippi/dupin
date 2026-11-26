import { exec, items as catalogue, storage } from 'dupin';
import type {
	ActionContext,
	ActionEffect,
	CatalogueItem,
	Item,
	ItemsContext,
	ItemsResult,
	Json,
	View,
	ViewContext,
} from 'dupin';
import {
	MAX_LINES,
	MAX_SOURCES,
	MIN_LINES,
	PATH,
	TERMINALS,
	UNIT,
	validHost,
	validPid,
	validTerminal,
} from './checks.ts';
import { processName } from './parse.ts';
import type { PortRow, ProcessRow } from './parse.ts';
import { limited, probesOf, refresh, run } from './probes.ts';
import { hostItem, hostRows } from './rows.ts';
import type { Fetched, RowRef } from './rows.ts';
import { configHosts } from './ssh-config.ts';
import {
	defaultHost,
	forget as forgetFetched,
	load,
	MAX_HOSTS,
	saveHost,
	saveSettings,
} from './store.ts';
import type { HostSettings, Probed, Stored } from './store.ts';
import { ago, clamp, fenced, line } from './text.ts';

/** The most rows the catalogue holds, well inside what Dupin keeps for one extension. */
const MAX_ITEMS = 4000;
const PART_ITEMS = 400;
const PART_CHARS = 700_000;
/** The most hosts one Refresh All fetches, so it ends inside an action's 30 seconds. */
const REFRESH_ALL_HOSTS = 6;
const PAGE_ROWS = 50;

function hostsOf(
	stored: Stored,
	configured: string[],
): { enabled: HostSettings[]; other: string[] } {
	const enabled = [...stored.hosts.values()]
		.filter((host) => host.enabled)
		.sort((a, b) => (a.host < b.host ? -1 : a.host > b.host ? 1 : 0));
	const on = new Set(enabled.map((host) => host.host));
	const other = [
		...new Set([...configured, ...stored.settings.hosts, ...stored.hosts.keys()]),
	].filter((host) => !on.has(host));
	return { enabled, other };
}

async function everything(): Promise<CatalogueItem[]> {
	const [stored, config] = await Promise.all([load(), configHosts()]);
	const { enabled, other } = hostsOf(stored, config.hosts);
	const fromConfig = new Set(config.hosts);
	const rows: CatalogueItem[] = enabled.map((host) => hostItem(host, stored.probes.get(host.host)));
	for (const host of other) {
		rows.push({
			id: `off:${host}`,
			title: host,
			subtitle: fromConfig.has(host) ? 'In your SSH config' : 'Added in Remote Settings',
			section: 'Other Hosts',
			icon: 'sf:server.rack',
			accessories: [{ tag: 'off' }],
			matchText: `${host} host`,
			actions: [
				{ title: `Enable ${host}`, run: 'enable', data: { host } },
				{ title: `Configure ${host}…`, push: { view: 'configure', state: { host } } },
				{ title: 'Copy Host Name', copy: host },
			],
		});
	}
	for (const problem of config.problems) {
		rows.push({
			id: `config:${rows.length}`,
			title: line(problem, 256),
			subtitle: 'Remote lists the hosts in ~/.ssh/config and those you add in Remote Settings',
			section: 'Remote',
			icon: 'sf:exclamationmark.triangle',
			matchText: 'settings remote ssh config',
			actions: [{ title: 'Remote Settings…', push: { view: 'settings' } }],
		});
	}
	rows.push({
		id: 'settings',
		title: 'Remote Settings…',
		subtitle: 'Terminal app and hosts beside those in ~/.ssh/config',
		section: 'Remote',
		icon: 'sf:gearshape.2',
		matchText: 'settings remote terminal hosts',
		actions: [{ title: 'Remote Settings…', push: { view: 'settings' } }],
	});
	if (enabled.length > 0) {
		rows.push({
			id: 'refresh-all',
			title: 'Refresh All Hosts',
			subtitle: `Fetches up to ${REFRESH_ALL_HOSTS} enabled hosts, least recently fetched first`,
			section: 'Remote',
			icon: 'sf:arrow.clockwise',
			matchText: 'refresh all hosts remote',
			actions: [{ title: 'Refresh All Hosts', run: 'refreshAll' }],
		});
	}
	if (stored.probes.size > 0) {
		rows.push({
			id: 'forget',
			title: 'Forget Fetched Results',
			subtitle: 'Deletes what Remote keeps of every host’s processes, ports, logs and files',
			section: 'Remote',
			icon: 'sf:trash',
			matchText: 'forget clear delete cache remote',
			actions: [
				{
					title: 'Forget Fetched Results',
					run: 'forget',
					confirm: 'Delete everything Remote fetched from your hosts?',
				},
			],
		});
	}
	for (const host of enabled) rows.push(...hostRows(host, stored.probes.get(host.host)));
	return rows.slice(0, MAX_ITEMS);
}

/**
 * The catalogue Dupin matches as the user types: the enabled hosts with how they answered last,
 * then everything last fetched from them. It reads only the package's storage and the SSH
 * configuration, never the network; fetching is an action the user chooses.
 */
export async function items(ctx: ItemsContext): Promise<ItemsResult> {
	const rows = await everything();
	const start = ctx.reason === 'more' ? Math.max(0, Number(ctx.cursor) || 0) : 0;
	let end = start;
	let chars = 0;
	while (end < rows.length && end - start < PART_ITEMS) {
		chars += JSON.stringify(rows[end]).length;
		if (chars > PART_CHARS && end > start) break;
		end++;
	}
	const part = rows.slice(start, end);
	const rest = end < rows.length ? { more: true, cursor: String(end) } : {};
	// Only the first part replaces what Dupin holds; the later ones add to it.
	return start === 0
		? { ttl: 60, items: part, ...rest }
		: { type: 'update', ttl: 60, append: part, ...rest };
}

function hostOf(data: Json): string | null {
	return validHost((data as { host?: unknown } | null)?.host);
}

function settingsFor(stored: Stored, host: string): HostSettings {
	return stored.hosts.get(host) ?? defaultHost(host);
}

function summary(host: string, result: { first: Probed; failed: number }): string {
	if (result.first.state !== 'ok' && result.first.message) {
		return line(`${host}: ${result.first.message}`, 256);
	}
	return result.failed === 0
		? `Refreshed ${host}`
		: `Refreshed ${host}; ${result.failed} ${result.failed === 1 ? 'command' : 'commands'} failed`;
}

function lines(text: unknown, pattern: RegExp): { values: string[]; bad: string | null } {
	const values = String(text ?? '')
		.split('\n')
		.map((value) => value.trim())
		.filter(Boolean);
	const bad = values.find((value) => !pattern.test(value)) ?? null;
	return { values: [...new Set(values)], bad };
}

function readRow(record: { data: Json } | undefined, ref: RowRef) {
	const data = record?.data as Partial<Probed> | null | undefined;
	if (
		!data ||
		data.host !== ref.host ||
		data.fetchedAt !== ref.fetchedAt ||
		!Array.isArray(data.rows)
	) {
		return null;
	}
	const row = data.rows[ref.index];
	return row === undefined ? null : { probe: String(data.probe ?? ''), row };
}

/** A catalogue row as a page's row, which Dupin doesn't date or match itself. */
function pageItem(row: CatalogueItem, now: number): Item {
	const {
		matchText: _match,
		time,
		keywords: _keywords,
		aliases: _aliases,
		file: _file,
		webSearch: _web,
		...rest
	} = row;
	return {
		...rest,
		subtitle: line(
			time === undefined ? (row.subtitle ?? '') : `${ago(now, time)} · ${row.subtitle ?? ''}`,
			2048,
		),
	};
}

export const views = {
	/**
	 * One host's rows, found again as the page's query changes: the same words that find them
	 * after `remote <host>` in the launcher.
	 */
	async host(ctx: ViewContext): Promise<View> {
		const state = ctx.state as { host?: unknown; records?: unknown; terminate?: unknown } | null;
		const host = validHost(state?.host);
		if (host === null) return { type: 'list', items: [], emptyText: 'No such host' };
		const ids = Array.isArray(state?.records)
			? state.records.filter((id) => typeof id === 'string')
			: [];
		const fetched: Fetched = new Map();
		for (const id of ids.slice(0, 64)) {
			const record = await storage.get(id as string);
			const data = record?.data as Partial<Probed> | null | undefined;
			if (record && data && data.host === host && typeof data.probe === 'string') {
				fetched.set(data.probe, { id: record.id, probed: data as Probed });
			}
		}
		const config: HostSettings = {
			...defaultHost(host),
			enabled: true,
			allowTerminate: state?.terminate === true,
		};
		for (const probed of fetched.values()) {
			const id = probed.probed.probe;
			if (id.startsWith('unit:')) config.units.push(id.slice(5));
			else if (id.startsWith('log:')) config.logs.push(id.slice(4));
			else if (id.startsWith('dir:')) config.dirs.push(id.slice(4));
		}
		const words = ctx.query.toLowerCase().split(/\s+/).filter(Boolean);
		const now = Date.now();
		const header = hostItem(config, fetched);
		const rows = hostRows(config, fetched).filter((row) => {
			const text = `${row.title} ${row.matchText ?? ''}`.toLowerCase();
			return words.every((word) => text.includes(word));
		});
		const first = fetched.get('ps')?.probed;
		const shown: Item[] = [
			{
				...pageItem(header, now),
				section: undefined,
				actions: [
					{ title: `Refresh ${host}`, run: 'refresh', data: { host } },
					{ title: 'Open SSH Session', run: 'terminal', data: { host }, alternate: true },
					{ title: 'Copy Host Name', copy: host },
				],
				...(first ? {} : { subtitle: 'Not fetched yet: ↩ fetches it' }),
			},
			...rows.slice(0, PAGE_ROWS - 1).map((row) => pageItem(row, now)),
		];
		return { type: 'list', items: shown, emptyText: `Nothing on ${host} matches` };
	},

	/** Everything Remote knows of one row, in full. */
	async details(ctx: ViewContext): Promise<View> {
		const ref = ctx.state as unknown as RowRef | null;
		const host = validHost(ref?.host);
		if (
			ref === null ||
			host === null ||
			typeof ref.record !== 'string' ||
			typeof ref.index !== 'number'
		) {
			return { type: 'list', items: [], emptyText: 'Nothing to show' };
		}
		const found = readRow(await storage.get(ref.record), ref);
		if (found === null) {
			return { type: 'list', items: [], emptyText: 'Refreshed since; search again to see it' };
		}
		const row = found.row as Partial<ProcessRow & PortRow & { line: string; path: string }>;
		const fetched = `${ago(Date.now(), ref.fetchedAt)} from ${host}`;
		if (row.pid !== undefined) {
			const name = line(processName(row as ProcessRow), 128);
			return {
				type: 'detail',
				title: line(`${name} (PID ${row.pid})`, 256),
				metadata: [
					{ label: 'Host', text: host },
					{ label: 'User', text: line(row.user ?? '', 512) || '—' },
					{ label: 'CPU', text: `${line(row.cpu ?? '', 16)}%` },
					{ label: 'Memory', text: `${line(row.mem ?? '', 16)}%` },
					{ label: 'Running for', text: line(row.elapsed ?? '', 64) || '—' },
					{ label: 'Fetched', text: fetched },
				],
				markdown: fenced(row.command ?? '', 8192),
				actions: [
					{ title: 'Copy Command Line', copy: row.command || String(row.pid) },
					{ title: 'Copy PID', copy: String(row.pid) },
				],
			};
		}
		if (row.local !== undefined) {
			return {
				type: 'detail',
				title: line(`${row.proto ?? ''} ${row.local}`, 256),
				metadata: [
					{ label: 'Host', text: host },
					{ label: 'State', text: line(row.state ?? '', 64) || '—' },
					{ label: 'Peer', text: line(row.peer ?? '', 512) || '—' },
					{ label: 'Fetched', text: fetched },
				],
				markdown: fenced(row.process ?? '', 8192),
				actions: [{ title: 'Copy Address', copy: row.local || ' ' }],
			};
		}
		const text = row.line ?? row.path ?? '';
		const source = found.probe.replace(/^(unit|log|dir):/, '');
		return {
			type: 'detail',
			title: line(row.path ?? source, 256),
			metadata: [
				{ label: 'Host', text: host },
				{ label: row.path === undefined ? 'Source' : 'Folder', text: line(source, 512) || '—' },
				{ label: 'Fetched', text: fetched },
			],
			markdown: fenced(text, 8192),
			actions: [{ title: 'Copy', copy: text || ' ' }],
		};
	},

	/** What Remote fetches from one host, and whether it may stop processes there. */
	async configure(ctx: ViewContext): Promise<View> {
		const host = validHost((ctx.state as { host?: unknown } | null)?.host);
		if (host === null) return { type: 'list', items: [], emptyText: 'No such host' };
		const config = settingsFor(await load(), host);
		return {
			type: 'form',
			title: `Configure ${host}`,
			submit: 'saveHost',
			submitTitle: 'Save',
			fields: [
				{ type: 'checkbox', id: 'enabled', title: 'Fetch from this host', default: config.enabled },
				{
					type: 'textarea',
					id: 'units',
					title: `systemd units whose last lines to show (up to ${MAX_SOURCES})`,
					default: config.units.join('\n'),
					placeholder: 'nginx.service',
				},
				{
					type: 'textarea',
					id: 'logs',
					title: `Log files to show the end of (up to ${MAX_SOURCES})`,
					default: config.logs.join('\n'),
					placeholder: '/var/log/syslog',
				},
				{
					type: 'textarea',
					id: 'dirs',
					title: `Folders to list, two levels deep (up to ${MAX_SOURCES})`,
					default: config.dirs.join('\n'),
					placeholder: '/srv/app',
				},
				{
					type: 'text',
					id: 'lines',
					title: `Lines of each log (${MIN_LINES}–${MAX_LINES})`,
					default: String(config.lines),
				},
				{
					type: 'checkbox',
					id: 'allowTerminate',
					title: 'Offer Terminate for this host’s processes',
					default: config.allowTerminate,
				},
			],
		};
	},

	async settings(): Promise<View> {
		const { settings } = await load();
		return {
			type: 'form',
			title: 'Remote Settings',
			submit: 'saveSettings',
			submitTitle: 'Save',
			fields: [
				{
					type: 'select',
					id: 'terminal',
					title: 'Open SSH sessions in',
					options: TERMINALS.map((name) => ({ value: name, title: name })),
					default: settings.terminal,
				},
				{
					type: 'textarea',
					id: 'hosts',
					title: 'Hosts beside those in ~/.ssh/config, one a line',
					default: settings.hosts.join('\n'),
					placeholder: 'deploy@build.example.com',
				},
			],
		};
	},
};

async function terminalFor(host: string): Promise<ActionEffect> {
	const { settings } = await load();
	try {
		const result = await exec.run('terminal', { app: settings.terminal, host });
		if (result.code === 0) return { type: 'hide' };
		return { type: 'toast', text: `${settings.terminal} didn’t open ssh://${host}` };
	} catch (error) {
		return {
			type: 'toast',
			text: line(error instanceof Error ? error.message : String(error), 256),
		};
	}
}

export const actions = {
	async refresh(data: Json, ctx: ActionContext): Promise<ActionEffect> {
		const host = hostOf(data);
		if (host === null) return { type: 'toast', text: 'No such host' };
		const stored = await load();
		const config = stored.hosts.get(host);
		if (!config?.enabled) return { type: 'toast', text: `Enable ${host} first` };
		const result = await refresh(config, 3, stored.probes.get(host));
		catalogue.invalidate();
		const page = ctx.state as { host?: unknown } | null;
		if (page?.host !== host) return { type: 'refresh', toast: summary(host, result) };
		// The host's page reads the records it was opened with, and a first fetch adds new ones.
		const fetched = (await load()).probes.get(host);
		return {
			type: 'replaceState',
			state: {
				...page,
				records: [...(fetched?.values() ?? [])].map((entry) => entry.id).slice(0, 64),
			} as Json,
			toast: summary(host, result),
		};
	},

	async refreshAll(): Promise<ActionEffect> {
		const stored = await load();
		const fetchedAt = (host: HostSettings) =>
			stored.probes.get(host.host)?.get('ps')?.probed.fetchedAt ?? 0;
		const hosts = [...stored.hosts.values()]
			.filter((host) => host.enabled)
			.sort((a, b) => fetchedAt(a) - fetchedAt(b));
		const chosen = hosts.slice(0, REFRESH_ALL_HOSTS);
		// Dupin runs four extension commands at once, so two hosts fetch two commands each.
		const results = await Promise.all(
			limited(
				chosen.map((host) => () => refresh(host, 2, stored.probes.get(host.host))),
				2,
			),
		);
		catalogue.invalidate();
		const down = results.filter((result) => result.first.state !== 'ok').length;
		const left = hosts.length - chosen.length;
		return {
			type: 'toast',
			text: `Refreshed ${chosen.length} ${chosen.length === 1 ? 'host' : 'hosts'}${down > 0 ? `, ${down} not answering` : ''}${left > 0 ? `; ${left} left for next time` : ''}`,
		};
	},

	async enable(data: Json): Promise<ActionEffect> {
		const host = hostOf(data);
		if (host === null) return { type: 'toast', text: 'No such host' };
		const stored = await load();
		if (!stored.hosts.has(host) && stored.hosts.size >= MAX_HOSTS) {
			return { type: 'toast', text: `Remote keeps settings for at most ${MAX_HOSTS} hosts` };
		}
		const config = { ...settingsFor(stored, host), enabled: true };
		await saveHost(config);
		const result = await refresh(config, 3, stored.probes.get(host));
		catalogue.invalidate();
		return { type: 'toast', text: line(`Enabled ${host}. ${summary(host, result)}`, 512) };
	},

	async terminal(data: Json): Promise<ActionEffect> {
		const host = hostOf(data);
		if (host === null) return { type: 'toast', text: 'No such host' };
		return terminalFor(host);
	},

	/** Sends TERM to one process, only on a host whose settings allow it, after Dupin's confirmation. */
	async terminate(data: Json): Promise<ActionEffect> {
		const host = hostOf(data);
		const pid = validPid((data as { pid?: unknown } | null)?.pid);
		if (host === null || pid === null) return { type: 'toast', text: 'No such process' };
		const stored = await load();
		const config = stored.hosts.get(host);
		if (!config?.enabled || !config.allowTerminate) {
			return {
				type: 'toast',
				text: `Terminate isn’t allowed on ${host}; turn it on in Configure ${host}`,
			};
		}
		let code: number | null;
		try {
			code = (await exec.run('terminate', { host, pid })).code;
		} catch (error) {
			return {
				type: 'toast',
				text: line(error instanceof Error ? error.message : String(error), 256),
			};
		}
		const processes = probesOf(config)[0];
		if (processes) await run(processes, config.lines, stored.probes.get(host)?.get('ps')?.probed);
		catalogue.invalidate();
		return {
			type: 'refresh',
			toast:
				code === 0
					? `Sent TERM to ${pid} on ${host}`
					: line(`kill exited with ${code ?? 'a signal'} on ${host}`, 256),
		};
	},

	async saveHost(values: Json, ctx: ActionContext): Promise<ActionEffect> {
		const host = validHost((ctx.state as { host?: unknown } | null)?.host);
		if (host === null) return { type: 'pop', toast: 'No such host' };
		const form = values as Record<string, unknown>;
		const errors: Record<string, string> = {};
		const units = lines(form.units, UNIT);
		const logs = lines(form.logs, PATH);
		const dirs = lines(form.dirs, PATH);
		if (units.bad !== null) errors.units = clamp(`“${line(units.bad, 64)}” isn’t a unit name`, 256);
		else if (units.values.length > MAX_SOURCES) errors.units = `At most ${MAX_SOURCES}`;
		for (const [id, list] of [
			['logs', logs],
			['dirs', dirs],
		] as const) {
			if (list.bad !== null) {
				errors[id] = clamp(
					`“${line(list.bad, 64)}” must be an absolute path of letters, digits and . _ / @ : + , = -`,
					256,
				);
			} else if (list.values.length > MAX_SOURCES) errors[id] = `At most ${MAX_SOURCES}`;
		}
		const count = Number(String(form.lines ?? '').trim());
		if (!Number.isInteger(count) || count < MIN_LINES || count > MAX_LINES) {
			errors.lines = `A whole number from ${MIN_LINES} to ${MAX_LINES}`;
		}
		if (Object.keys(errors).length > 0) return { type: 'fieldErrors', errors };
		const stored = await load();
		if (!stored.hosts.has(host) && stored.hosts.size >= MAX_HOSTS) {
			return { type: 'pop', toast: `Remote keeps settings for at most ${MAX_HOSTS} hosts` };
		}
		const config: HostSettings = {
			host,
			enabled: form.enabled === true,
			units: units.values,
			logs: logs.values,
			dirs: dirs.values,
			lines: count,
			allowTerminate: form.allowTerminate === true,
		};
		await saveHost(config);
		const kept = new Set(config.enabled ? probesOf(config).map((probe) => probe.id) : []);
		await forgetFetched(stored, host, kept);
		catalogue.invalidate();
		return { type: 'pop', toast: `Saved ${host}` };
	},

	async saveSettings(values: Json): Promise<ActionEffect> {
		const form = values as Record<string, unknown>;
		const hosts = String(form.hosts ?? '')
			.split('\n')
			.map((host) => host.trim())
			.filter(Boolean);
		const bad = hosts.find((host) => validHost(host) === null);
		if (bad !== undefined) {
			return {
				type: 'fieldErrors',
				errors: { hosts: clamp(`“${line(bad, 64)}” isn’t a host name or user@host`, 256) },
			};
		}
		if (hosts.length > MAX_HOSTS) {
			return { type: 'fieldErrors', errors: { hosts: `At most ${MAX_HOSTS} hosts` } };
		}
		await saveSettings({ terminal: validTerminal(form.terminal), hosts: [...new Set(hosts)] });
		catalogue.invalidate();
		return { type: 'pop', toast: 'Saved Remote Settings' };
	},

	async forget(): Promise<ActionEffect> {
		await forgetFetched(await load(), null);
		catalogue.invalidate();
		return { type: 'toast', text: 'Forgot everything Remote fetched' };
	},
};
