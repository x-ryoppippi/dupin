import { repo } from '../site.ts';

// The pages of docs/ the site publishes, by section, as paths relative to docs/. Pages for
// people working on Dupin itself (development, architecture, design) stay on GitHub, and links
// to them go there. What is native and what is a bundled extension follows the built-ins table
// in docs/design/extension-platform.md.
export const sections = [
	{
		title: 'Getting started',
		pages: ['usage.md', 'tutorial.md', 'configuration.md', 'features/keywords.md'],
	},
	{ title: 'About', pages: ['philosophy.md', 'name.md', 'faq.md'] },
	{
		title: 'Built-in features',
		intro: 'Native to Dupin and always there.',
		pages: [
			'features/apps.md',
			'features/recent-items.md',
			'features/files.md',
			'features/folder-browsing.md',
			'features/file-actions.md',
			'features/send-to.md',
			'features/snapshots.md',
			'features/scripts.md',
			'features/hotkeys.md',
			'features/selected-text.md',
			'features/system-commands.md',
			'features/settings.md',
			'features/appearance.md',
			'features/shaders.md',
			'features/permissions.md',
			'features/storage.md',
			'features/sync.md',
			'features/updates.md',
			'features/telemetry.md',
			'features/advanced.md',
		],
	},
	{
		title: 'Bundled extensions',
		intro: 'Shipped with Dupin as extensions, each one yours to eject and change.',
		pages: [
			'features/calculator.md',
			'features/calendar.md',
			'features/clipboard.md',
			'features/dictionary.md',
			'features/emoji.md',
			'features/repo.md',
			'features/snippets.md',
			'features/web-searches.md',
			'features/maps.md',
			'features/window-management.md',
			'features/processes.md',
			'features/ports.md',
			'features/coding-agents.md',
		],
	},
	{
		title: 'Your own extensions',
		intro: 'Write, install and test extensions of your own.',
		pages: [
			'features/extensions.md',
			'features/extension-manifest.md',
			'features/extension-api.md',
			'features/extension-permissions.md',
			'features/extension-testing.md',
			'features/extension-distribution.md',
		],
	},
	{
		title: 'Scripts and agents',
		intro: 'Drive Dupin from a terminal, a script or an AI agent.',
		pages: ['features/cli.md', 'features/mcp.md'],
	},
] as const satisfies readonly { title: string; intro?: string; pages: readonly string[] }[];

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
