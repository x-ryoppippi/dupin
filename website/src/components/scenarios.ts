// The searches the demo types, with what Dupin lists after each key, written out by hand: the
// list narrows as the query grows, as it does in the app.

export type Row = {
	/** An `app-icon--*` look; `glyph` draws a Phosphor icon on a `tint`ed square. */
	icon: string;
	glyph?: string;
	tint?: string;
	title: string;
	sub: string;
	/** The name the query matched in place of the title, shown before the location. */
	alias?: string;
};

/** How a mode marks what matched: apps by letters in order, files by the typed run, contents not at all. */
export type Marking = 'letters' | 'run' | 'none';

export type Scenario = {
	mode?: 'Files' | 'In files' | 'Define';
	keyword?: string;
	detail?: string;
	actions?: boolean;
	query: string;
	/** The rows after each prefix of `query`. */
	steps: Record<string, Row[]>;
};

const app = (title: string, sub: string, icon: string, extra: Partial<Row> = {}): Row => ({
	icon,
	title,
	sub,
	...extra,
});
const glyph = (title: string, sub: string, glyph: string, tint: string): Row => ({
	icon: 'glyph',
	glyph,
	tint,
	title,
	sub,
});

const calculator = glyph(
	'Calculator',
	'/System/Applications',
	'icon-[ph--calculator-fill]',
	'#3a3a3c',
);
const calendar = glyph(
	'Calendar',
	'/System/Applications',
	'icon-[ph--calendar-blank-fill]',
	'#ff3b30',
);
const chatgpt = app('ChatGPT', '/Applications', 'chatgpt');
const codex = app('ChatGPT', '/Applications', 'chatgpt', { alias: 'Codex' });
const chess = glyph('Chess', '/System/Applications', 'icon-[ph--horse-fill]', '#8d6e4f');
const claude = app('Claude', '/Applications', 'claude');
const clock = app('Clock', '/System/Applications', 'clock');
const colorsync = glyph(
	'ColorSync Utility',
	'/System/Applications/Utilities',
	'icon-[ph--palette-fill]',
	'#5e5ce6',
);
const console_ = glyph(
	'Console',
	'/System/Applications/Utilities',
	'icon-[ph--warning-fill]',
	'#2c2c2e',
);
const contacts = glyph(
	'Contacts',
	'/System/Applications',
	'icon-[ph--address-book-fill]',
	'#a2845e',
);
const t3code = app('T3 Code', '/Applications', 't3code');
const terminal = glyph(
	'Terminal',
	'/System/Applications/Utilities',
	'icon-[ph--terminal-fill]',
	'#1c1c1e',
);
const textedit = glyph(
	'TextEdit',
	'/System/Applications',
	'icon-[ph--note-pencil-fill]',
	'#8e8e93',
);
const tips = glyph('Tips', '/System/Applications', 'icon-[ph--lightbulb-fill]', '#f5b300');
const tv = glyph('TV', '/System/Applications', 'icon-[ph--television-simple-fill]', '#1c1c1e');

const activity = glyph(
	'Activity Monitor',
	'/System/Applications/Utilities',
	'icon-[ph--pulse-fill]',
	'#1c1c1e',
);
const fontbook = glyph('Font Book', '/System/Applications', 'icon-[ph--text-aa-fill]', '#8e8e93');
const shortcuts = glyph('Shortcuts', '/System/Applications', 'icon-[ph--stack-fill]', '#e5486b');
const finder = glyph('Finder', '/System/Library/CoreServices', 'icon-[ph--smiley-fill]', '#1e88f5');
const imageCapture = glyph(
	'Image Capture',
	'/System/Applications',
	'icon-[ph--camera-fill]',
	'#636366',
);
const iphone = glyph(
	'iPhone Mirroring',
	'/System/Applications',
	'icon-[ph--device-mobile-fill]',
	'#3a3a3c',
);
const mission = glyph(
	'Mission Control',
	'/System/Applications',
	'icon-[ph--squares-four-fill]',
	'#5e5ce6',
);
const music = glyph('Music', '/System/Applications', 'icon-[ph--music-note-fill]', '#fa2d48');
const itunes = { ...music, alias: 'iTunes' };

const file = (title: string, sub: string, icon: 'pdf' | 'doc' | 'sheet'): Row => ({
	icon,
	title,
	sub,
});
const ideas = file('ideas.md', '~/Notes', 'doc');
const index = file('index.html', '~/Sites/blog', 'doc');
const install = file('install.sh', '~/Developer/dotfiles', 'doc');
const interview = file('interview-notes.md', '~/Notes', 'doc');
const inventory = file('inventory.numbers', '~/Documents/Shop', 'sheet');
const invitation = file('invitation.pdf', '~/Downloads', 'pdf');
const invoice10 = file('invoice-2026-10.pdf', '~/Documents/Invoices', 'pdf');
const invoice09 = file('invoice-2026-09.pdf', '~/Documents/Invoices', 'pdf');
const invoices = file('invoices.numbers', '~/Documents', 'sheet');
const template = file('invoice-template.pages', '~/Documents/Templates', 'doc');

const trip = file('trip-notes.md', '~/Notes', 'doc');
const booking = file('lake-tahoe-booking.pdf', '~/Downloads', 'pdf');
const packing = file('packing-list.md', '~/Notes', 'doc');
const budget = file('travel-budget.numbers', '~/Documents', 'sheet');
const offsite = file('team-offsite.md', '~/Documents', 'doc');
const taxes = file('taxes-2025.numbers', '~/Documents', 'sheet');

const c = [calculator, calendar, chatgpt, chess, claude, clock, console_, contacts];
const invo = [invoice10, invoice09, invoices, template];
const tah = [trip, booking, packing, budget];
const meeting = file('meeting-notes.md', '~/Notes', 'doc');
const reading = file('reading-list.md', '~/Notes', 'doc');

const scenarios = {
	apps: {
		query: 'cl',
		steps: { c, cl: [claude, clock, calculator, calendar, console_, colorsync] },
	},
	codex: {
		query: 'codex',
		steps: {
			c,
			co: [console_, contacts, colorsync, codex, clock, calculator],
			cod: [codex],
			code: [codex],
			codex: [codex],
		},
	},
	t3: {
		query: 't3',
		steps: {
			t: [tv, tips, t3code, terminal, textedit, shortcuts, fontbook, activity],
			t3: [t3code],
		},
	},
	files: {
		mode: 'Files',
		actions: true,
		query: 'invoice',
		steps: {
			i: [index, ideas, install, interview, inventory, invitation, invoice10, invoice09],
			in: [index, install, interview, inventory, invitation, invoice10, invoice09, invoices],
			inv: [inventory, invitation, invoice10, invoice09, invoices, template],
			invo: invo,
			invoi: invo,
			invoic: invo,
			invoice: invo,
		},
	},
	contents: {
		mode: 'In files',
		query: 'tahoe',
		steps: {
			// `in` is typed in apps mode, before Space turns it into the mode keyword.
			i: [imageCapture, iphone, finder, music, mission],
			in: [iphone, finder, itunes, mission],
			t: [trip, offsite, taxes, budget, meeting, packing, reading, booking],
			ta: [taxes, trip, budget, booking, packing],
			tah: tah,
			taho: tah,
			tahoe: tah,
		},
	},
} satisfies Record<string, Scenario>;

export const marking = (mode: Scenario['mode']): Marking =>
	mode === 'Files' ? 'run' : mode ? 'none' : 'letters';

export const features = [
	{ label: 'Apps', scenario: scenarios.apps },
	{ label: 'Files', scenario: scenarios.files },
	{ label: 'In files', scenario: scenarios.contents },
	{
		label: 'Dictionary',
		scenario: {
			mode: 'Define',
			keyword: 'define',
			query: 'serendipity',
			detail: 'The occurrence of events by chance in a happy or beneficial way.',
			steps: {},
		},
	},
] satisfies { label: string; scenario: Scenario }[];

/** `title` split into runs, marking the characters `query` matched as `marking` says. */
export function marked(
	title: string,
	query: string,
	marking: Marking,
): { text: string; hit: boolean }[] {
	const chars = title.split('');
	const hits = new Set<number>();
	const lower = chars.map((ch) => ch.toLowerCase());
	const q = query.toLowerCase().split('');
	if (marking === 'run') {
		const at = title.toLowerCase().indexOf(query.toLowerCase());
		if (at >= 0) q.forEach((_, k) => hits.add(at + k));
	} else if (marking === 'letters') {
		let from = 0;
		for (const ch of q) {
			const at = lower.indexOf(ch, from);
			if (at < 0) break;
			hits.add(at);
			from = at + 1;
		}
	}
	const runs: { text: string; hit: boolean }[] = [];
	chars.forEach((ch, k) => {
		const hit = hits.has(k);
		const last = runs.at(-1);
		if (last && last.hit === hit) last.text += ch;
		else runs.push({ text: ch, hit });
	});
	return runs;
}
