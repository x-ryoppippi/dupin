import { apps, items as catalogue } from 'dupin';
import type { Action, ActionEffect, Actions, CatalogueItem, RunningApp } from 'dupin';

/** Asked again this often while the keyword shows the list. */
const TTL_SECONDS = 2;

/** An app, as a kind action receives it from an app row or a row here names it. */
type Target = { path: string } | { pid: number };

function target(app: RunningApp): Target {
	return app.path ? { path: app.path } : { pid: app.pid };
}

function row(app: RunningApp): CatalogueItem {
	const data = target(app);
	const actions: Action[] = [
		{ title: 'Quit', run: 'quit', data },
		{ title: 'Hide', run: 'hide', data },
		{
			title: 'Force Quit…',
			run: 'quit_forcibly',
			data,
			confirm: `Force Quit “${app.name}”?`,
		},
	];
	if (app.path) actions.push({ title: 'Reveal in Finder', reveal: app.path });
	const accessories: CatalogueItem['accessories'] = [];
	if (app.active) accessories.push({ tag: 'Active' });
	if (app.hidden) accessories.push({ tag: 'Hidden' });
	return {
		// A bundle ID keeps a row's ID, and a shortcut bound to it, across launches.
		id: app.bundleId ?? String(app.pid),
		title: app.name,
		subtitle: app.bundleId ?? `PID ${app.pid}`,
		...(app.path ? { file: app.path } : {}),
		keywords: app.bundleId ? [app.bundleId] : [],
		accessories,
		actions,
	};
}

/** The apps with a place in the Dock, as macOS lists them, Dupin left out. */
export async function items() {
	const running = await apps.running();
	return { items: running.map(row), ttl: TTL_SECONDS };
}

function toast(text: string): ActionEffect {
	return { type: 'toast', text };
}

const gone = toast('That app is no longer running');

export const actions: Actions = {
	async hide(data) {
		const outcome = await apps.hide(data as Target);
		if (!outcome) return gone;
		catalogue.invalidate();
		return toast(outcome.done ? `Hid ${outcome.app.name}` : `${outcome.app.name} didn't hide`);
	},
	async quit(data) {
		const outcome = await apps.quit(data as Target);
		if (!outcome) return gone;
		catalogue.invalidate();
		return toast(
			outcome.done
				? `Quit ${outcome.app.name}`
				: `${outcome.app.name} hasn't quit yet; it may be asking you something`,
		);
	},
	async quit_forcibly(data) {
		const outcome = await apps.forceQuit(data as Target);
		if (!outcome) return gone;
		catalogue.invalidate();
		return toast(
			outcome.done ? `Forced ${outcome.app.name} to quit` : `${outcome.app.name} didn't quit`,
		);
	},
};
