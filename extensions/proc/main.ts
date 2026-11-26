import { exec, items as catalogue } from 'dupin';
import type { Action, Actions, CatalogueItem, ItemsContext, ItemsResult, Views } from 'dupin';
import {
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
const TTL_SECONDS = 2;
/** Rows in one part of the catalogue, well inside the bytes an answer may carry. */
const PART = 400;

interface Target {
	pid: number;
	name: string;
	path: string;
}

function row(
	process: Process,
	command: string | undefined,
	uid: number | null,
	now: number,
): CatalogueItem {
	const target: Target = { pid: process.pid, name: process.name, path: process.path };
	const actions: Action[] = [
		{ title: 'Details', run: 'details', data: { ...target, user: process.user } },
		{ title: 'Copy PID', copy: String(process.pid) },
	];
	if (process.path.startsWith('/'))
		actions.push({ title: 'Reveal in Finder', reveal: process.path });
	actions.push(
		{ title: 'Open Activity Monitor', run: 'activityMonitor' },
		{ title: 'Show Open Ports', run: 'showPorts', data: { pid: process.pid } },
	);
	if (stoppable(process, uid)) {
		actions.push(
			{
				title: 'Terminate',
				run: 'terminate',
				data: { ...target },
				confirm: `Terminate ${process.name}?`,
			},
			{
				title: 'Force Quit',
				run: 'forceQuit',
				data: { ...target },
				confirm: `Force ${process.name} to quit?`,
			},
		);
	}
	return {
		id: String(process.pid),
		title: process.name,
		subtitle: cut(`${process.pid} · ${process.user} · ${command ?? process.path}`, 160),
		keywords: [String(process.pid), process.user],
		time: now - process.elapsed,
		accessories: [
			{ text: `${process.cpu.toFixed(1)}%` },
			{ text: memory(process.rss) },
			{ freshness: now },
		],
		actions,
	};
}

/**
 * Every process, in parts of {@link PART} by PID: each part lists the processes after the last
 * PID the one before it sent, so a process that starts or ends between parts never repeats one.
 */
export async function items(ctx: ItemsContext): Promise<ItemsResult> {
	const after = ctx.reason === 'more' ? Number(ctx.cursor ?? 0) : 0;
	const processes = parseProcesses(await run('processes'));
	const uid = ownUid(processes);
	const part = processes.filter((process) => process.pid > after).slice(0, PART);
	const commands = part.length
		? parseCommands(await run('commands', { pids: part.map((process) => process.pid).join(',') }))
		: new Map<number, string>();
	const now = Date.now();
	const rows = part.map((process) => row(process, commands.get(process.pid), uid, now));
	const last = part.at(-1)?.pid ?? after;
	const more = processes.some((process) => process.pid > last);
	const answer = { ttl: TTL_SECONDS, cursor: String(last), more };
	return ctx.reason === 'more'
		? { type: 'update', append: rows, ...answer }
		: { items: rows, ...answer };
}

interface Details extends Target {
	user: string;
	command: string;
	folder: string | null;
}

export const views: Views = {
	details(ctx) {
		const details = ctx.state as unknown as Details;
		const target: Target = { pid: details.pid, name: details.name, path: details.path };
		const actions: Action[] = [
			{ title: 'Copy PID', copy: String(details.pid) },
			{ title: 'Copy Command', copy: details.command || details.path },
			{ title: 'Show Open Ports', run: 'showPorts', data: { pid: details.pid } },
		];
		if (details.path.startsWith('/'))
			actions.push({ title: 'Reveal in Finder', reveal: details.path });
		actions.push({
			title: 'Terminate',
			run: 'terminate',
			data: { ...target },
			confirm: `Terminate ${details.name}?`,
		});
		const fence = details.command.includes('```') ? '~~~~' : '```';
		return {
			type: 'detail',
			title: details.name,
			metadata: [
				{ label: 'PID', text: String(details.pid) },
				{ label: 'User', text: details.user },
				{ label: 'Program', text: cut(details.path, 512) },
				...(details.folder ? [{ label: 'Folder', text: cut(details.folder, 512) }] : []),
			],
			markdown: `${fence}\n${details.command}\n${fence}`,
			actions,
		};
	},
};

export const actions: Actions = {
	/** Opens Ports with the PID typed, which lists what the process listens on. */
	showPorts(data) {
		const { pid } = data as { pid: number };
		return { type: 'enter', command: 'port', query: String(pid) };
	},
	/** Reads the whole command line and working folder, which the list leaves out, and shows them. */
	async details(data) {
		const target = data as unknown as Target & { user: string };
		const pid = String(target.pid);
		const command = parseCommands(await run('commands', { pids: pid })).get(target.pid);
		if (command === undefined)
			return { type: 'toast', text: `${target.name} (${pid}) has already ended` };
		const listed = await exec.run('cwd', { pids: pid });
		const folder = parseFolders(listed.stdout ?? '').get(target.pid) ?? null;
		const details: Details = { ...target, command: cut(command, 12 * 1024), folder };
		return { type: 'push', view: 'details', title: target.name, state: { ...details } };
	},
	async activityMonitor() {
		await run('activity-monitor');
		return { type: 'hide' };
	},
	async terminate(data) {
		const text = await signal('terminate', data as unknown as Target);
		catalogue.invalidate();
		return { type: 'toast', text };
	},
	async forceQuit(data) {
		const text = await signal('force-quit', data as unknown as Target);
		catalogue.invalidate();
		return { type: 'toast', text };
	},
};
