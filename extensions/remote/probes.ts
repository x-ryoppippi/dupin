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
	const description = (error as { description?: unknown } | null)?.description;
	if (code === 'needs-consent' || code === 'revoked' || code === 'denied' || code === 'disabled') {
		return {
			state: 'consent',
			message: line(
				typeof description === 'string'
					? `Allow Remote to ${description} in Settings → Extensions`
					: 'Allow Remote to run ssh in Settings → Extensions',
				256,
			),
		};
	}
	return {
		state: 'failed',
		message: line(error instanceof Error ? error.message : String(error), 256),
	};
}

/** Runs `probe` and keeps what it found in the package's storage. */
export async function run(probe: Probe, lines: number): Promise<Probed> {
	const host = probe.inputs.host ?? '';
	let probed: Probed;
	try {
		const result = await exec.run(probe.profile, probe.inputs);
		const ended = outcome(result);
		probed = {
			host,
			probe: probe.id,
			fetchedAt: Date.now(),
			...ended,
			truncated: result.truncated === true,
			rows: unanswered(ended.state) ? [] : rowsOf(probe, result.stdout ?? '', lines),
		};
	} catch (error) {
		probed = { host, probe: probe.id, fetchedAt: Date.now(), ...refused(error), rows: [] };
	}
	await saveProbed(probed);
	return probed;
}

/** At most `limit` of `tasks` at once, so a refresh never queues more requests than Dupin takes. */
export async function limited<T>(tasks: (() => Promise<T>)[], limit: number): Promise<T[]> {
	const results: T[] = [];
	let next = 0;
	const worker = async () => {
		while (next < tasks.length) {
			const index = next++;
			const task = tasks[index];
			if (task) results[index] = await task();
		}
	};
	await Promise.all(Array.from({ length: Math.min(limit, tasks.length) }, worker));
	return results;
}

/**
 * Fetches everything `config` asks for. Processes go first: when the host doesn't answer, the
 * rest would only wait out the same timeout, so they are left as they were.
 */
export async function refresh(
	config: HostSettings,
	concurrency: number,
): Promise<{ first: Probed; failed: number }> {
	const [first, ...rest] = probesOf(config);
	const processes = await run(first as Probe, config.lines);
	if (unanswered(processes.state)) {
		return { first: processes, failed: 1 };
	}
	const others = await limited(
		rest.map((probe) => () => run(probe, config.lines)),
		concurrency,
	);
	return {
		first: processes,
		failed: [processes, ...others].filter((probed) => probed.state !== 'ok').length,
	};
}
