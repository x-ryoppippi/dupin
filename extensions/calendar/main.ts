import { calendar, DupinError } from 'dupin';
import type { CalendarEvent, Item, List, SearchContext, Section } from 'dupin';

const MAX_ROWS = 30;
const UPCOMING_DAYS = 7;
const PAST_DAYS = 30;
const FUTURE_DAYS = 90;
const MAX_ID_BYTES = 128;
/** The longest query the calendar service matches. */
const MAX_QUERY_CHARS = 200;
const EARLIER = 'Earlier';

/** The same wall-clock time `days` later, across daylight saving changes. */
function daysAfter(date: Date, days: number): Date {
	const moved = new Date(date.getTime());
	moved.setDate(moved.getDate() + days);
	return moved;
}

/** The local day `time` falls on, counted from the epoch. */
function dayOf(time: number): number {
	const date = new Date(time);
	return Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()) / 864e5;
}

interface Placed {
	event: CalendarEvent;
	start: number;
	end: number;
}

async function read(range: Parameters<typeof calendar.events>[0]): Promise<Placed[]> {
	try {
		return (await calendar.events(range)).map((event) => ({
			event,
			start: Date.parse(event.start),
			end: Date.parse(event.end),
		}));
	} catch (error) {
		// Dupin lists the request to allow above the results.
		if (error instanceof DupinError && error.code === 'needs-consent') return [];
		throw error;
	}
}

function row(placed: Placed, ended: boolean, heading: number): Item {
	const { event, start } = placed;
	const time =
		!ended && event.labels.time !== null && dayOf(start) === heading
			? event.labels.time
			: event.labels.dates;
	const item: Item = {
		title: event.title.trim() === '' ? 'Untitled event' : event.title,
		subtitle: `${time} · ${event.calendar.name}`,
	};
	if (event.id.length <= MAX_ID_BYTES) item.id = event.id;
	if (event.link !== null) item.actions = [{ title: 'Open in Calendar', open: event.link }];
	return item;
}

export async function search(query: string, ctx: SearchContext): Promise<List> {
	const now = new Date();
	const typed = query.trim() !== '';
	const limit = Math.min(ctx.limit, MAX_ROWS);
	const words = Array.from(query.trim()).slice(0, MAX_QUERY_CHARS).join('');
	const [upcoming, past] = await Promise.all([
		read({
			from: now,
			to: daysAfter(now, typed ? FUTURE_DAYS : UPCOMING_DAYS),
			query: words,
			limit,
		}),
		// The events still under way are in both reads. Ended ones are shown only while fewer
		// than `limit` are upcoming, so fewer than `limit` are under way, and this read's
		// `limit` holds them and every ended one that is shown.
		typed
			? read({ from: daysAfter(now, -PAST_DAYS), to: now, query: words, limit, order: 'latest' })
			: [],
	]);
	const left = Math.max(0, limit - upcoming.length);
	const ended = past
		.filter(({ start, end }) => start < now.getTime() && end <= now.getTime())
		.sort(
			(a, b) =>
				b.start - a.start || (a.event.id < b.event.id ? -1 : a.event.id > b.event.id ? 1 : 0),
		);
	const today = dayOf(now.getTime());
	const sections: Section[] = [];
	const add = (title: string, item: Item) => {
		const last = sections.at(-1);
		if (last?.title === title) last.items.push(item);
		else sections.push({ title, items: [item] });
	};
	for (const each of upcoming) {
		// An event already under way is listed under today.
		const heading = dayOf(Math.max(each.start, now.getTime()));
		const title =
			heading === today ? 'Today' : heading === today + 1 ? 'Tomorrow' : each.event.labels.day;
		add(title, row(each, false, heading));
	}
	for (const each of ended.slice(0, left)) {
		add(EARLIER, row(each, true, today));
	}
	return { sections };
}
