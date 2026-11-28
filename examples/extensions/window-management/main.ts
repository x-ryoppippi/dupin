import { DupinError, windows } from 'dupin';
import type { ActionEffect, Actions, CatalogueItem, WindowDisplay, WindowFrame } from 'dupin';

type Layout =
	| 'left-half'
	| 'right-half'
	| 'top-half'
	| 'bottom-half'
	| 'left-third'
	| 'centre-third'
	| 'right-third'
	| 'maximise'
	| 'centre'
	| 'next-display';

/** Each command's id and title, and the other spellings that find it. */
const COMMANDS: readonly (readonly [Layout, string, readonly string[]])[] = [
	['left-half', 'Left Half', []],
	['right-half', 'Right Half', []],
	['top-half', 'Top Half', []],
	['bottom-half', 'Bottom Half', []],
	['left-third', 'Left Third', []],
	['centre-third', 'Centre Third', ['Center Third']],
	['right-third', 'Right Third', []],
	['maximise', 'Maximise', ['Maximize']],
	['centre', 'Centre', ['Center']],
	['next-display', 'Next Display', []],
];

export function items(): CatalogueItem[] {
	return COMMANDS.map(([id, title, spellings]) => ({
		id,
		title,
		subtitle: 'Window Management',
		keywords: [
			...spellings,
			...[title, ...spellings].map((name) => `window ${name.toLowerCase()}`),
			// The feature's name lists every command, as the native search did.
			'Window Management',
		],
		actions: [{ title: 'Arrange', run: 'arrange', data: id }],
	}));
}

function overlap(a: WindowFrame, b: WindowFrame): number {
	const width = Math.min(a.x + a.width, b.x + b.width) - Math.max(a.x, b.x);
	const height = Math.min(a.y + a.height, b.y + b.height) - Math.max(a.y, b.y);
	return Math.max(width, 0) * Math.max(height, 0);
}

/** One cell of `area` split into `columns` × `rows`, with edges on whole points so adjacent cells meet. */
function tile(
	area: WindowFrame,
	column: number,
	columns: number,
	row: number,
	rows: number,
): WindowFrame {
	const edge = (length: number, index: number, count: number) =>
		Math.round((length * index) / count);
	const x = edge(area.width, column, columns);
	const y = edge(area.height, row, rows);
	return {
		x: area.x + x,
		y: area.y + y,
		width: edge(area.width, column + 1, columns) - x,
		height: edge(area.height, row + 1, rows) - y,
	};
}

/** Where `layout` puts `window`, on the display it overlaps most, or `null` without a display. */
function targetFrame(
	layout: Layout,
	window: WindowFrame,
	displays: readonly WindowDisplay[],
): WindowFrame | null {
	let index = -1;
	let most = -1;
	displays.forEach((display, at) => {
		const area = overlap(display.frame, window);
		if (area > most) {
			index = at;
			most = area;
		}
	});
	if (index < 0) return null;
	const visible = displays[index].visibleFrame;
	switch (layout) {
		case 'left-half':
			return tile(visible, 0, 2, 0, 1);
		case 'right-half':
			return tile(visible, 1, 2, 0, 1);
		case 'top-half':
			return tile(visible, 0, 1, 0, 2);
		case 'bottom-half':
			return tile(visible, 0, 1, 1, 2);
		case 'left-third':
			return tile(visible, 0, 3, 0, 1);
		case 'centre-third':
			return tile(visible, 1, 3, 0, 1);
		case 'right-third':
			return tile(visible, 2, 3, 0, 1);
		case 'maximise':
			return { ...visible };
		case 'centre': {
			const width = Math.min(window.width, visible.width);
			const height = Math.min(window.height, visible.height);
			return {
				x: visible.x + (visible.width - width) / 2,
				y: visible.y + (visible.height - height) / 2,
				width,
				height,
			};
		}
		case 'next-display': {
			if (displays.length === 1) return { ...window };
			// Keep the window's place relative to the usable area, shrinking it only to fit.
			const next = displays[(index + 1) % displays.length].visibleFrame;
			const width = Math.min(window.width, next.width);
			const height = Math.min(window.height, next.height);
			const fraction = (offset: number, travel: number) =>
				travel > 0 ? Math.min(Math.max(offset / travel, 0), 1) : 0.5;
			const x = fraction(window.x - visible.x, visible.width - window.width);
			const y = fraction(window.y - visible.y, visible.height - window.height);
			return {
				x: next.x + x * (next.width - width),
				y: next.y + y * (next.height - height),
				width,
				height,
			};
		}
	}
}

const toast = (text: string): ActionEffect => ({ type: 'toast', text });

export const actions: Actions = {
	async arrange(data) {
		const command = COMMANDS.find(([id]) => id === data);
		if (command === undefined) return toast('That window command no longer exists.');
		try {
			const [displays, focused] = await Promise.all([windows.displays(), windows.focused()]);
			if (focused === null) return toast('There is no focused window to arrange.');
			const target = targetFrame(command[0], focused.frame, displays);
			if (target === null) return toast('No display is available to arrange this window.');
			await windows.setFrame(target);
			return { type: 'hide' };
		} catch (error) {
			if (!(error instanceof DupinError)) throw error;
			// Dupin lists the request to allow above the results.
			if (error.code === 'needs-consent') return;
			return toast(error.message);
		}
	},
};
