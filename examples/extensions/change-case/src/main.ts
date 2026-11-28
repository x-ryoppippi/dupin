import { camelCase, constantCase, kebabCase, pascalCase, snakeCase } from 'change-case';
import type { Item, SearchContext } from 'dupin';

const CASES = [
	['camelCase', camelCase],
	['PascalCase', pascalCase],
	['snake_case', snakeCase],
	['kebab-case', kebabCase],
	['CONSTANT_CASE', constantCase],
] as const;

// Dupin refuses the whole answer when one row breaks a limit, so a long conversion keeps its
// full text for Copy and shows a shortened title.
const MAX_TITLE_BYTES = 512;
const MAX_TEXT_BYTES = 16 * 1024;

export function search(query: string, _ctx: SearchContext): Item[] {
	const input = query.trim();
	if (!input) return [];
	return CASES.flatMap(([name, convert]) => {
		const converted = convert(input);
		if (!converted || utf8Length(converted) > MAX_TEXT_BYTES) return [];
		return [{ title: clip(converted, MAX_TITLE_BYTES), subtitle: name, text: converted }];
	});
}

function utf8Length(text: string): number {
	let bytes = 0;
	for (const char of text) bytes += charBytes(char);
	return bytes;
}

function charBytes(char: string): number {
	const point = char.codePointAt(0) ?? 0;
	return point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4;
}

/** `text`, or as much of it as fits in `limit` bytes with an ellipsis. */
function clip(text: string, limit: number): string {
	if (utf8Length(text) <= limit) return text;
	const ellipsis = '…';
	let budget = limit - charBytes(ellipsis);
	let clipped = '';
	for (const char of text) {
		budget -= charBytes(char);
		if (budget < 0) break;
		clipped += char;
	}
	return clipped + ellipsis;
}
