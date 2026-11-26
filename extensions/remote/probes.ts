import { exec } from 'dupin';
import type { ExecResult } from 'dupin';
import { logLines, paths, ports, processes } from './parse.ts';
import type { Row } from './parse.ts';
import { saveProbed } from './store.ts';
import type { HostSettings, Probed, State } from './store.ts';
import { line } from './text.ts';

/** One fixed command run on a host, by the exec profile that declares it. */
export interface Probe {
	id: string;
	/** The section its rows are listed under, after the host's name. */
	title: string;
	kind: 'processes' | 'ports' | 'log' | 'files';
	profile: string;
	inputs: Record<string, string>;
}

/** What `config` asks to fetch from its host, processes first, since they tell whether it answers. */
export function probesOf(config: HostSettings): Probe[] {
	const host = config.host;
	const lines = String(config.lines);
	return [
		{ id: 'ps', title: 'Processes', kind: 'processes', profile: 'processes', inputs: { host } },
		{ id: 'ports', title: 'Ports', kind: 'ports', profile: 'ports', inputs: { host } },
		...config.units.map((unit): Probe => ({
			id: `unit:${unit}`,
			title: unit,
			kind: 'log',
			profile: 'journal',
			inputs: { host, unit, lines },
		})),
		...config.logs.map((path): Probe => ({
			id: `log:${path}`,
			title: path,
			kind: 'log',
			profile: 'log-file',
			inputs: { host, path, lines },
		})),
		...config.dirs.map((dir): Probe => ({
			id: `dir:${dir}`,
			title: dir,
			kind: 'files',
			profile: 'files',
			inputs: { host, dir },
		})),
	];
}

/** Whether `state` means the host itself didn't answer, so its other probes needn't run. */
export function unanswered(state: State): boolean {
	return state === 'unreachable' || state === 'auth' || state === 'host-key' || state === 'timeout';
}

function firstLine(text: string | undefined): string {
	const found = (text ?? '').split('\n').find((part) => part.trim() !== '') ?? '';
	return line(found, 256);
}

/** How ssh, or the command it ran, ended. `ssh` exits 255 when it couldn't run the command. */
function outcome(result: ExecResult): { state: State; message?: string } {
	if (result.timedOut) return { state: 'timeout', message: 'Timed out' };
	if (result.code === 255) {
		const stderr = result.stderr ?? '';
		if (/Permission denied|Too many authentication failures/.test(stderr)) {
			return { state: 'auth', message: 'SSH refused the keys your agent offered' };
		}
		if (
			/Host key verification failed|HOST IDENTIFICATION HAS CHANGED|No .* host key is known/.test(
				stderr,
			)
		) {
			return { state: 'host-key', message: 'Its host key isn’t known; connect once in Terminal' };
		}
		return { state: 'unreachable', message: firstLine(stderr) || 'SSH couldn’t connect' };
	}
	if (result.code === null) return { state: 'failed', message: 'Stopped by a signal' };
	if (result.code !== 0) {
		return { state: 'failed', message: firstLine(result.stderr) || `Exited with ${result.code}` };
	}
	return { state: 'ok' };
}

function rowsOf(probe: Probe, stdout: string, lines: number): Row[] {
	switch (probe.kind) {
		case 'processes':
			return processes(stdout);
		case 'ports':
			return ports(stdout);
		case 'log':
			return logLines(stdout, lines);
		case 'files':
			return paths(stdout);
	}
}

function refused(error: unknown): { state: State; message: string } {
	const code = (error as { code?: unknown } | null)?.code;
	const scope = (error as { scope?: unknown } | null)?.scope;
	if (code === 'needs-consent' || code === 'revoked' || code === 'denied' || code === 'disabled') {
		return {
			state: 'consent',
			message: line(
				code === 'disabled'
					? 'Turn Remote on in Settings → Extensions'
					: `Allow ${typeof scope === 'string' ? scope : 'ssh'} in Settings → Extensions`,
				256,
			),
		};
	}
	return {
		state: 'failed',
		message: line(error instanceof Error ? error.message : String(error), 256),
	};
}

/**
 * Runs `probe`. When the host doesn't answer, the rows `previous` fetched stay, with when they
 * were fetched, so they still show, marked.
 */
async function fetchProbe(probe: Probe, lines: number, previous?: Probed): Promise<Probed> {
	const host = probe.inputs.host ?? '';
	const now = Date.now();
	let probed: Probed;
	try {
		const result = await exec.run(probe.profile, probe.inputs);
		const ended = outcome(result);
		probed = unanswered(ended.state)
			? { host, probe: probe.id, fetchedAt: now, ...ended, rows: [] }
			: {
					host,
					probe: probe.id,
					fetchedAt: now,
					...ended,
					truncated: result.truncated === true,
					rows: rowsOf(probe, result.stdout ?? '', lines),
				};
	} catch (error) {
		probed = { host, probe: probe.id, fetchedAt: now, ...refused(error), rows: [] };
	}
	if (probed.state !== 'ok' && probed.rows.length === 0 && previous && previous.rows.length > 0) {
		probed = { ...probed, fetchedAt: previous.fetchedAt, triedAt: now, rows: previous.rows };
	}
	return probed;
}

/** Runs `probe` and keeps what it found in the package's storage. */
export async function run(probe: Probe, lines: number, previous?: Probed): Promise<Probed> {
	const probed = await fetchProbe(probe, lines, previous);
	await saveProbed(probed);
	return probed;
}

/**
 * Starts `tasks`, at most `limit` at once, so a refresh never asks for more commands than Dupin
 * runs together, and returns each one's promise in the order of `tasks`.
 */
export function limited<T>(tasks: (() => Promise<T>)[], limit: number): Promise<T>[] {
	let active = 0;
	const waiting: (() => void)[] = [];
	return tasks.map(async (task) => {
		if (active >= limit) await new Promise<void>((resolve) => waiting.push(resolve));
		active++;
		try {
			return await task();
		} finally {
			active--;
			waiting.shift()?.();
		}
	});
}

/**
 * Fetches everything `config` asks for. Processes go first: when the host doesn't answer, the
 * rest would only wait out the same timeout, so they are left as they were. The rest run side by
 * side and are kept in the order the settings list them, as each one and those before it end.
 */
export async function refresh(
	config: HostSettings,
	concurrency: number,
	previous?: Map<string, { probed: Probed }>,
): Promise<{ first: Probed; failed: number }> {
	const [first, ...rest] = probesOf(config);
	const before = (probe: Probe) => previous?.get(probe.id)?.probed;
	const processes = await run(first as Probe, config.lines, before(first as Probe));
	if (unanswered(processes.state)) {
		return { first: processes, failed: 1 };
	}
	const pending = limited(
		rest.map((probe) => () => fetchProbe(probe, config.lines, before(probe))),
		concurrency,
	);
	const others: Probed[] = [];
	for (const next of pending) {
		const probed = await next;
		await saveProbed(probed);
		others.push(probed);
	}
	return {
		first: processes,
		failed: [processes, ...others].filter((probed) => probed.state !== 'ok').length,
	};
}
