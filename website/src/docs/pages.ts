import { repo } from '../site.ts';

// The pages of docs/ the site publishes, by section, as paths relative to docs/. Pages for
// people working on Dupin itself (development, architecture, design) stay on GitHub, and links
// to them go there.
export const sections = [
	{ title: 'Getting started', pages: ['usage.md', 'configuration.md', 'features/keywords.md'] },
	{
		title: 'Searching',
		pages: [
			'features/apps.md',
			'features/recent-items.md',
			'features/files.md',
			'features/folder-browsing.md',
			'features/file-actions.md',
			'features/send-to.md',
			'features/dictionary.md',
			'features/web-searches.md',
			'features/maps.md',
			'features/calculator.md',
			'features/calendar.md',
			'features/clipboard.md',
			'features/snippets.md',
			'features/emoji.md',
			'features/snapshots.md',
		],
	},
	{
		title: 'Commands',
		pages: [
			'features/scripts.md',
			'features/hotkeys.md',
			'features/selected-text.md',
			'features/window-management.md',
			'features/system-commands.md',
		],
	},
	{
		title: 'The app',
		pages: [
			'features/settings.md',
			'features/appearance.md',
			'features/permissions.md',
			'features/storage.md',
			'features/updates.md',
		],
	},
	{
		title: 'Extending',
		pages: ['features/extensions.md', 'features/advanced.md', 'features/mcp.md'],
	},
	{ title: 'About', pages: ['name.md'] },
] as const satisfies readonly { title: string; pages: readonly string[] }[];

export type DocPath = (typeof sections)[number]['pages'][number];

export const docPaths: readonly DocPath[] = sections.flatMap((section) => section.pages);

const published = new Set<string>(docPaths);

export const isPublished = (path: string): path is DocPath => published.has(path);

// usage.md is the front page of the docs; every other page sits at its path without `.md`.
export const docRoute = (path: DocPath) =>
	path === 'usage.md' ? '/docs/' : `/docs/${path.replace(/\.md$/, '')}/`;

// The page's Markdown twin, as the build writes it beside the HTML.
export const markdownRoute = (path: DocPath) =>
	path === 'usage.md' ? '/docs.md' : `/docs/${path.replace(/\.md$/, '')}.md`;

export const githubUrl = (repoPath: string, kind: 'blob' | 'tree' = 'blob') =>
	`${repo}/${kind}/main/${repoPath}`;

// The page's title, its first heading.
export const docTitle = (source: string) => /^# (.+)$/m.exec(source)?.[1].trim() ?? 'Dupin';
