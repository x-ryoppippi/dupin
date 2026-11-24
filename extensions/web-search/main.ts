import type { CatalogueItem, ItemsContext } from 'dupin';

interface Search {
	name: string;
	url: string;
}

const defaults: Record<string, Search> = {
	g: { name: 'Google', url: 'https://www.google.com/search?q={query}' },
	gh: { name: 'GitHub', url: 'https://github.com/search?q={query}' },
	maps: { name: 'Apple Maps', url: 'maps://?q={query}' },
	yt: { name: 'YouTube', url: 'https://www.youtube.com/results?search_query={query}' },
};

function isSearch(value: unknown): value is Search {
	return (
		typeof value === 'object' &&
		value !== null &&
		'name' in value &&
		typeof value.name === 'string' &&
		'url' in value &&
		typeof value.url === 'string'
	);
}

// Dupin hands over `web_searches` and `web_fallbacks` from config.toml only once checked, and
// only the keys that are set.
function configured(ctx: ItemsContext): [string, Search][] {
	const searches = ctx.preferences.webSearches;
	if (typeof searches !== 'object' || searches === null || Array.isArray(searches)) {
		return Object.entries(defaults);
	}
	const listed: [string, Search][] = [];
	for (const [alias, search] of Object.entries(searches)) {
		if (isSearch(search)) listed.push([alias, search]);
	}
	return listed;
}

function fallbacks(ctx: ItemsContext): string[] {
	const listed = ctx.preferences.webFallbacks;
	return Array.isArray(listed)
		? listed.filter((alias): alias is string => typeof alias === 'string')
		: [];
}

export function items(ctx: ItemsContext): CatalogueItem[] {
	const offered = fallbacks(ctx);
	return configured(ctx).map(([alias, { name, url }]) => {
		const fallback = offered.indexOf(alias);
		return {
			id: alias,
			title: `Search ${name}`,
			subtitle: alias,
			webSearch: {
				alias,
				url,
				title: `Search ${name} for {query}`,
				...(fallback >= 0 ? { fallback } : {}),
			},
		};
	});
}
