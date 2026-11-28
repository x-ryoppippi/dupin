import { exec, items as catalogue } from 'dupin';
import type { Action, Actions, CatalogueItem, ItemsResult } from 'dupin';

/** Asked again this often while the keyword shows the list. */
const TTL_SECONDS = 2;
/** Rows Dupin takes in one answer; a Mac rarely listens on more. */
const MAX_ROWS = 500;

/** One port a process listens on, over one protocol, at every address it does. */
interface Listener {
	protocol: 'TCP' | 'UDP';
	port: number;
	pid: number;
	command: string;
	addresses: string[];
}

interface Target {
	pid: number;
	name: string;
}

/** The address and port of an lsof `n` field, such as `*:3000`, `127.0.0.1:53` or `[::1]:8080`. */
function endpoint(name: string): { address: string; port: number } | null {
	// A connected or unbound socket listens on nothing.
	if (name.includes('->')) return null;
	const colon = name.lastIndexOf(':');
	const port = Number(name.slice(colon + 1));
	if (colon < 0 || !Number.isInteger(port) || port <= 0) return null;
	return { address: name.slice(0, colon), port };
}

/** Listening sockets from `lsof -F cnPpu`, merged by protocol, port and process. */
export function parseListeners(stdout: string): Listener[] {
	const listeners = new Map<string, Listener>();
	let pid = 0;
	let command = '';
	let protocol = '';
	for (const line of stdout.split('\n')) {
		const field = line[0];
		const value = line.slice(1);
		if (field === 'p') pid = Number(value);
		else if (field === 'c') command = value;
		else if (field === 'f') protocol = '';
		else if (field === 'P') protocol = value;
		else if (field === 'n' && (protocol === 'TCP' || protocol === 'UDP')) {
			const found = endpoint(value);
			if (!found) continue;
			const key = `${protocol}-${found.port}-${pid}`;
			const listener = listeners.get(key) ?? {
				protocol,
				port: found.port,
				pid,
				command,
				addresses: [],
			};
			if (!listener.addresses.includes(found.address)) listener.addresses.push(found.address);
			listeners.set(key, listener);
		}
	}
	return [...listeners.values()].sort(
		(a, b) => a.port - b.port || a.protocol.localeCompare(b.protocol) || a.pid - b.pid,
	);
}

function row(listener: Listener, read: number): CatalogueItem {
	const { protocol, port, pid, command } = listener;
	const target: Target = { pid, name: command };
	const actions: Action[] = [];
	if (protocol === 'TCP')
		actions.push({ title: 'Open in Browser', open: `http://localhost:${port}` });
	actions.push(
		{ title: 'Copy Address', copy: `localhost:${port}` },
		{ title: 'Copy Port', copy: String(port) },
		{
			title: 'Terminate',
			run: 'terminate',
			data: { ...target },
			confirm: `Terminate ${command}, which listens on ${port}?`,
		},
	);
	return {
		id: `${protocol}-${port}-${pid}`,
		title: String(port),
		subtitle: `${command} · ${pid} · ${listener.addresses.join(', ')}`,
		keywords: [command, String(pid)],
		accessories: [{ tag: protocol }, { freshness: read }],
		actions,
	};
}

/** Every port a process of the user's listens on. */
export async function items(): Promise<ItemsResult> {
	const listed = await exec.run('sockets');
	const read = Date.now();
	// lsof exits 1 when it finds no socket at all.
	if (listed.code !== 0 && listed.code !== 1)
		throw new Error(`lsof exited ${listed.code ?? 'on a signal'}`);
	const rows = parseListeners(listed.stdout ?? '')
		.slice(0, MAX_ROWS)
		.map((listener) => row(listener, read));
	return { items: rows, ttl: TTL_SECONDS };
}

export const actions: Actions = {
	async terminate(data) {
		const { pid, name } = data as unknown as Target;
		if (!Number.isInteger(pid) || pid <= 1)
			return { type: 'toast', text: `Dupin doesn't signal ${name}` };
		const identified = await exec.run('identify', { pid: String(pid) });
		const path = (identified.stdout ?? '').trim();
		const program = path.slice(path.lastIndexOf('/') + 1);
		if (identified.code !== 0) return { type: 'toast', text: `${name} (${pid}) has already ended` };
		if (program.toLowerCase() === 'dupin' || path.includes('/Dupin.app/')) {
			return { type: 'toast', text: "Dupin doesn't quit itself" };
		}
		if (!program.startsWith(name) && !name.startsWith(program)) {
			return {
				type: 'toast',
				text: `PID ${pid} now belongs to another program, so Dupin left it alone`,
			};
		}
		const sent = await exec.run('terminate', { pid: String(pid) });
		catalogue.invalidate();
		return {
			type: 'toast',
			text:
				sent.code === 0
					? `Asked ${name} (${pid}) to quit`
					: `Couldn't signal ${name} (${pid}): it isn't yours, or it ended`,
		};
	},
};
