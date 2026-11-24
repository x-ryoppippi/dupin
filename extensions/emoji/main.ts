import type { Action, CatalogueItem, ItemsContext, ItemsResult } from 'dupin';
import { rows, sections } from './catalogue.ts';

const PART = 500;

const TONES = ['light', 'medium-light', 'medium', 'medium-dark', 'dark'];

function words(field: string): string[] {
	return field === '' ? [] : field.split('|');
}

function codePoints(char: string): string {
	return Array.from(char, (point) => {
		const code = point.codePointAt(0) ?? 0;
		return `U+${code.toString(16).toUpperCase().padStart(4, '0')}`;
	}).join(' ');
}

function item(row: string): CatalogueItem {
	const [char, name, section, en, ja, japanese, tones] = row.split('\t');
	const title = `${char} ${name.charAt(0).toUpperCase()}${name.slice(1)}`;
	const keywords = [...words(en), ...words(ja)];
	const actions: Action[] = [
		{ title: 'Copy', copy: char },
		{ title: 'Paste', paste: char },
		...words(tones).map((variant, index) => ({
			title: `Copy ${variant} ${TONES[index]} skin tone`,
			copy: variant,
		})),
	];
	const metadata = [
		{ label: 'Character', text: char },
		{ label: 'Name', text: name },
		...(japanese === '' ? [] : [{ label: '日本語', text: japanese }]),
		{ label: 'Code points', text: codePoints(char) },
	];
	if (keywords.length > 0) metadata.push({ label: 'Keywords', text: keywords.join(', ') });
	return {
		id: codePoints(char).replaceAll('U+', '').replaceAll(' ', '-').toLowerCase(),
		title,
		section: sections[Number(section)],
		keywords: [name, ...keywords],
		detail: { metadata },
		actions,
	};
}

export function items(ctx: ItemsContext): ItemsResult {
	const start = ctx.reason === 'more' ? Number(ctx.cursor) : 0;
	const end = Math.min(start + PART, rows.length);
	const part = rows.slice(start, end).map(item);
	const more = end < rows.length;
	return start === 0
		? { items: part, cursor: String(end), more }
		: { type: 'update', append: part, cursor: String(end), more };
}
