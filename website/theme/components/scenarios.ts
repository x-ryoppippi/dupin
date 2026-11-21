export type Row = { icon: string; title: string; sub: string; hit?: string; alias?: string };
export type Scenario = { mode?: string; query: string; rows: Row[] };

// PanelDemo renders the first scenario; motion.js reads them all from data-scenarios to cycle.
export const scenarios: [Scenario, ...Scenario[]] = [
	{
		query: 'cl',
		rows: [
			{ icon: 'claude', title: 'Claude', sub: '/Applications', hit: 'Cl' },
			{ icon: 'clock', title: 'Clock', sub: '/System/Applications', hit: 'Cl' },
		],
	},
	{
		query: 'codex',
		rows: [{ icon: 'chatgpt', title: 'ChatGPT', sub: '/Applications', alias: 'Codex' }],
	},
	{
		query: 't3',
		rows: [{ icon: 't3code', title: 'T3 Code', sub: '/Applications', hit: 'T3' }],
	},
	{
		mode: 'Files',
		query: 'invoice',
		rows: [
			{ icon: 'pdf', title: 'invoice-2026-10.pdf', sub: '~/Documents/Invoices', hit: 'invoice' },
			{ icon: 'pdf', title: 'invoice-2026-09.pdf', sub: '~/Documents/Invoices', hit: 'invoice' },
			{ icon: 'sheet', title: 'invoices.numbers', sub: '~/Documents', hit: 'invoice' },
		],
	},
	{
		mode: 'In files',
		query: 'tahoe',
		rows: [
			{ icon: 'doc', title: 'trip-notes.md', sub: '~/Notes' },
			{ icon: 'pdf', title: 'lake-tahoe-booking.pdf', sub: '~/Downloads' },
		],
	},
];
