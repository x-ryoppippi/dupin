import { exec, items as catalogue } from 'dupin';
import type { Action, Actions, CatalogueItem, ItemsResult } from 'dupin';
import { AGENTS, INTERPRETERS } from './agents.ts';
import {
	basename,
	cut,
	memory,
	ownUid,
	parseCommands,
	parseFolders,
	parseProcesses,
	run,
	signal,
	stoppable,
} from './ps.ts';
import type { Process } from './ps.ts';

/** Asked again this often while the keyword shows the list. */
const TTL_SECONDS = 5;
const MAX_ROWS = 200;

interface Agent {
	process: Process;
	/** The agent's name, as {@link AGENTS} lists it. */
	agent: string;
	folder: string | null;
}

function isInterpreter(name: string): boolean {
	return INTERPRETERS.some(
		(interpreter) => name === interpreter || name.startsWith(`${interpreter}.`),
	);
}

/** The agent an interpreter's command line runs, by its script's or module's name. */
export function scriptAgent(command: string): string | null {
	const words = command.split(/\s+/u).slice(1);
	for (let index = 0; index < words.length; index++) {
		const word = words[index];
		if (word === '-m') return named(words[index + 1] ?? '');
		if (word.startsWith('-')) continue;
		return named(basename(word).replace(/\.(c|m)?js$|\.py$/u, ''));
	}
	return null;
}

function named(name: string): string | null {
	return AGENTS.includes(name) ? name : null;
}

/** The agents among `processes`, leaving out the helpers an agent starts under its own name. */
export function agents(
	processes: readonly Process[],
	commands: ReadonlyMap<number, string>,
): Map<number, string> {
	const found = new Map<number, string>();
	for (const process of processes) {
		const agent = isInterpreter(process.name)
			? scriptAgent(commands.get(process.pid) ?? '')
			: named(process.name);
		if (agent) found.set(process.pid, agent);
	}
	for (const process of processes) {
		const agent = found.get(process.pid);
		if (agent && found.get(process.ppid) === agent) found.delete(process.pid);
	}
	return found;
}

function shown(folder: string, user: string): string {
	const home = `/Users/${user}`;
	return folder === home
		? '~'
		: folder.startsWith(`${home}/`)
			? `~${folder.slice(home.length)}`
			: folder;
}

function row({ process, agent, folder }: Agent, uid: number | null, now: number): CatalogueItem {
	const actions: Action[] = [];
	if (folder) {
		actions.push(
			{ title: 'Reveal Project in Finder', reveal: folder },
			{ title: 'Open Project in Terminal', run: 'terminal', data: { folder } },
			{ title: 'Copy Path', copy: folder },
		);
	}
	actions.push({ title: 'Show Process', run: 'showProcess', data: { pid: process.pid } });
	if (stoppable(process, uid)) {
		const target = { pid: process.pid, name: process.name, path: process.path };
		actions.push({
			title: 'Terminate',
			run: 'terminate',
			data: target,
			confirm: `Terminate ${agent}?`,
		});
	}
	const project = folder ? basename(folder) : 'Working folder unknown';
	return {
		id: String(process.pid),
		title: `${agent} · ${project}`,
		subtitle: cut(`${folder ? shown(folder, process.user) : process.path} · ${process.pid}`, 256),
		keywords: [agent, String(process.pid), ...(folder ? [project] : [])],
		time: now - process.elapsed,
		accessories: [
			{ text: `${process.cpu.toFixed(1)}%` },
			{ text: memory(process.rss) },
			{ freshness: now },
		],
		actions,
	};
}

/** The coding agents running on this Mac, newest first, with the folder each works in. */
export async function items(): Promise<ItemsResult> {
	const processes = parseProcesses(await run('processes'));
	const scripts = processes.filter((process) => isInterpreter(process.name));
	const commands = scripts.length
		? parseCommands(
				await run('commands', { pids: scripts.map((process) => process.pid).join(',') }),
			)
		: new Map<number, string>();
	const found = agents(processes, commands);
	const pids = [...found.keys()];
	const folders = pids.length
		? parseFolders((await exec.run('cwd', { pids: pids.join(',') })).stdout ?? '')
		: new Map<number, string>();
	const uid = ownUid(processes);
	const now = Date.now();
	const rows = processes
		.filter((process) => found.has(process.pid))
		.sort((a, b) => a.elapsed - b.elapsed)
		.slice(0, MAX_ROWS)
		.map((process) =>
			row(
				{
					process,
					agent: found.get(process.pid) ?? process.name,
					folder: folders.get(process.pid) ?? null,
				},
				uid,
				now,
			),
		);
	return { items: rows, ttl: TTL_SECONDS };
}

export const actions: Actions = {
	/** Opens Processes with the PID typed, which lists the agent's process. */
	showProcess(data) {
		const { pid } = data as { pid: number };
		return { type: 'enter', command: 'proc', query: String(pid) };
	},
	async terminal(data) {
		const { folder } = data as { folder: string };
		const opened = await exec.run('terminal', { folder });
		if (opened.code !== 0)
			return { type: 'toast', text: `Couldn't open ${basename(folder)} in Terminal` };
		return { type: 'hide' };
	},
	async terminate(data) {
		const text = await signal(
			'terminate',
			data as unknown as { pid: number; name: string; path: string },
		);
		catalogue.invalidate();
		return { type: 'toast', text };
	},
};
