import { line } from './text.ts';

export const MAX_PROCESSES = 300;
export const MAX_PORTS = 100;
export const MAX_FILES = 200;
/** No line of output is looked at past this many characters. */
const MAX_LINE_CHARS = 4096;

export interface ProcessRow {
	pid: string;
	user: string;
	cpu: string;
	mem: string;
	elapsed: string;
	command: string;
}

export interface PortRow {
	proto: string;
	state: string;
	local: string;
	peer: string;
	process: string;
}

export type Row = ProcessRow | PortRow | { line: string } | { path: string };

function linesOf(output: string): string[] {
	return output
		.split('\n')
		.map((text) => text.slice(0, MAX_LINE_CHARS))
		.filter((text) => text.trim() !== '');
}

/** `ps -A -o pid= -o user= -o pcpu= -o pmem= -o etime= -o args=` */
export function processes(output: string): ProcessRow[] {
	const rows: ProcessRow[] = [];
	for (const text of linesOf(output)) {
		const match = /^\s*(\d{1,10})\s+(\S+)\s+([\d.,]+)\s+([\d.,]+)\s+(\S+)\s?(.*)$/.exec(text);
		if (!match) continue;
		rows.push({
			pid: match[1] ?? '',
			user: line(match[2] ?? '', 64),
			cpu: line(match[3] ?? '', 16),
			mem: line(match[4] ?? '', 16),
			elapsed: line(match[5] ?? '', 32),
			command: line(match[6] ?? '', 1024),
		});
		if (rows.length >= MAX_PROCESSES) break;
	}
	return rows;
}

/** `ss -H -l -n -t -u -p`: Netid, State, Recv-Q, Send-Q, local and peer address, then the process. */
export function ports(output: string): PortRow[] {
	const rows: PortRow[] = [];
	for (const text of linesOf(output)) {
		const fields = text.trim().split(/\s+/);
		if (fields.length < 6) continue;
		rows.push({
			proto: line(fields[0] ?? '', 16),
			state: line(fields[1] ?? '', 16),
			local: line(fields[4] ?? '', 256),
			peer: line(fields[5] ?? '', 256),
			process: line(fields.slice(6).join(' '), 512),
		});
		if (rows.length >= MAX_PORTS) break;
	}
	return rows;
}

/** The last `count` lines `journalctl` or `tail` printed. */
export function logLines(output: string, count: number): { line: string }[] {
	return linesOf(output)
		.slice(-count)
		.map((text) => ({ line: line(text, 1024) }));
}

/** `find DIR -xdev -mindepth 1 -maxdepth 2 -type f`: one path a line. */
export function paths(output: string): { path: string }[] {
	return linesOf(output)
		.slice(0, MAX_FILES)
		.map((text) => ({ path: line(text, 1024) }));
}

/** The process's name from its command line, as a row's title shows it. */
export function processName(row: ProcessRow): string {
	const first = (row.command.split(' ')[0] ?? '').replace(/:$/, '');
	return first.split('/').pop() || row.command || `PID ${row.pid}`;
}
