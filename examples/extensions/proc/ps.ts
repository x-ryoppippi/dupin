import { exec } from 'dupin';

/** One process as `ps -o pid=,ppid=,uid=,user=,%cpu=,rss=,etime=,comm=` reports it. */
export interface Process {
	pid: number;
	ppid: number;
	uid: number;
	user: string;
	cpu: number;
	/** Resident memory in KiB. */
	rss: number;
	/** Milliseconds since it started. */
	elapsed: number;
	/** The program it runs, by the path it was started with. */
	path: string;
	name: string;
}

const PROCESS = /^\s*(\d+)\s+(\d+)\s+(\d+)\s+(\S+)\s+([\d.,]+)\s+(\d+)\s+(\S+)\s+(.+)$/u;

/** `ps`'s `etime`, `[[dd-]hh:]mm:ss`, in milliseconds. */
export function elapsed(text: string): number {
	const [days, clock] = text.includes('-') ? text.split('-') : ['0', text];
	const parts = clock.split(':').map(Number);
	while (parts.length < 3) parts.unshift(0);
	const [hours, minutes, seconds] = parts;
	return (((Number(days) * 24 + hours) * 60 + minutes) * 60 + seconds) * 1000;
}

export function basename(path: string): string {
	const trimmed = path.replace(/\/+$/u, '');
	return trimmed.slice(trimmed.lastIndexOf('/') + 1) || trimmed;
}

export function parseProcesses(stdout: string): Process[] {
	const processes: Process[] = [];
	for (const line of stdout.split('\n')) {
		const match = PROCESS.exec(line);
		if (!match) continue;
		const path = match[8].trim();
		processes.push({
			pid: Number(match[1]),
			ppid: Number(match[2]),
			uid: Number(match[3]),
			user: match[4],
			cpu: Number(match[5].replace(',', '.')),
			rss: Number(match[6]),
			elapsed: elapsed(match[7]),
			path,
			name: basename(path),
		});
	}
	return processes.sort((a, b) => a.pid - b.pid);
}

/** Each process's command line by PID, from `ps -o pid=,args=`. */
export function parseCommands(stdout: string): Map<number, string> {
	const commands = new Map<number, string>();
	for (const line of stdout.split('\n')) {
		const match = /^\s*(\d+)\s(.*)$/u.exec(line);
		if (match) commands.set(Number(match[1]), match[2].trim());
	}
	return commands;
}

/** Each process's working folder by PID, from `lsof -a -d cwd -F n`. */
export function parseFolders(stdout: string): Map<number, string> {
	const folders = new Map<number, string>();
	let pid = 0;
	for (const line of stdout.split('\n')) {
		if (line.startsWith('p')) pid = Number(line.slice(1));
		else if (line.startsWith('n') && pid > 0) folders.set(pid, line.slice(1));
	}
	return folders;
}

/** Dupin itself, and the extension hosts it starts, which are its own executable. */
export function isDupin(process: Pick<Process, 'name' | 'path'>): boolean {
	return process.name.toLowerCase() === 'dupin' || process.path.includes('/Dupin.app/');
}

/**
 * The user Dupin runs as: the owner of the `ps` this listing ran, which Dupin started, or of
 * Dupin itself.
 */
export function ownUid(processes: readonly Process[]): number | null {
	const dupin = new Set(processes.filter(isDupin).map((process) => process.pid));
	const listing = processes.find(
		(process) => process.path === '/bin/ps' && dupin.has(process.ppid),
	);
	return listing?.uid ?? processes.find(isDupin)?.uid ?? null;
}

/** Whether Dupin offers to signal it: the user's own, never launchd and never Dupin. */
export function stoppable(process: Process, uid: number | null): boolean {
	return process.pid > 1 && uid !== null && process.uid === uid && !isDupin(process);
}

export async function run(profile: string, inputs: Record<string, string> = {}): Promise<string> {
	const result = await exec.run(profile, inputs);
	if (result.code !== 0) throw new Error(`${profile} exited ${result.code ?? 'on a signal'}`);
	return result.stdout ?? '';
}

export function cut(text: string, length: number): string {
	return text.length > length ? `${text.slice(0, length - 1)}…` : text;
}

export function memory(kib: number): string {
	if (kib >= 1024 * 1024) return `${(kib / 1024 / 1024).toFixed(1)} GB`;
	if (kib >= 1024) return `${Math.round(kib / 1024)} MB`;
	return `${kib} KB`;
}

/** Signals `pid` through `profile`, once `identify` shows it still runs `path`. */
export async function signal(
	profile: 'terminate' | 'force-quit',
	target: { pid: number; name: string; path: string },
): Promise<string> {
	const { pid, name, path } = target;
	if (!Number.isInteger(pid) || pid <= 1 || isDupin({ name, path })) {
		return `Dupin doesn't signal ${name}`;
	}
	const identified = await exec.run('identify', { pid: String(pid) });
	if (identified.code !== 0) return `${name} (${pid}) has already ended`;
	if ((identified.stdout ?? '').trim() !== path) {
		return `PID ${pid} now belongs to another program, so Dupin left it alone`;
	}
	const sent = await exec.run(profile, { pid: String(pid) });
	if (sent.code !== 0) return `Couldn't signal ${name} (${pid}): it isn't yours, or it ended`;
	return `${profile === 'terminate' ? 'Asked' : 'Forced'} ${name} (${pid}) to quit`;
}
