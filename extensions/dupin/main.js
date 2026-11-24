// @ts-check

const repository = 'https://github.com/x-ryoppippi/dupin';

/** @type {import('dupin').Item[]} */
const rows = [
	{
		title: 'Using Dupin',
		subtitle: 'Search, keywords, actions and the panel',
		url: `${repository}/blob/main/docs/usage.md`,
	},
	{
		title: 'Settings and config.toml',
		subtitle: 'Every setting and its default',
		url: `${repository}/blob/main/docs/configuration.md`,
	},
	{
		title: 'Extensions',
		subtitle: 'Write, install and eject extensions',
		url: `${repository}/blob/main/docs/features/extensions.md`,
	},
	{
		title: 'Report a problem',
		subtitle: 'Open an issue on GitHub',
		url: `${repository}/issues/new`,
	},
	{
		title: 'Source code',
		subtitle: repository,
		url: repository,
	},
	{
		title: 'Website',
		subtitle: 'dupin.app',
		url: 'https://dupin.app',
	},
];

/** @type {import('dupin').Search} */
export const search = (query) => {
	const words = query.toLowerCase().split(/\s+/u).filter(Boolean);
	return rows.filter((row) => {
		const text = `${row.title} ${row.subtitle ?? ''}`.toLowerCase();
		return words.every((word) => text.includes(word));
	});
};
