import { camelCase, constantCase, kebabCase, pascalCase, snakeCase } from 'change-case';
import type { Item, SearchContext } from 'dupin';

const CASES = [
	['camelCase', camelCase],
	['PascalCase', pascalCase],
	['snake_case', snakeCase],
	['kebab-case', kebabCase],
	['CONSTANT_CASE', constantCase],
] as const;

export function search(query: string, _ctx: SearchContext): Item[] {
	const input = query.trim();
	if (!input) return [];
	return CASES.flatMap(([name, convert]) => {
		const converted = convert(input);
		return converted ? [{ title: converted, subtitle: name, text: converted }] : [];
	});
}
