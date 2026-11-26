import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import type {
	OxContentCustomHostModule,
	OxContentCustomHostRenderContext,
	OxContentCustomHostRoute,
} from '@ox-content/vite-plugin/custom-host';
import { render } from 'svelte/server';
import type { Component } from 'svelte';
import Docs from '../pages/Docs.svelte';
import NotFound from '../pages/NotFound.svelte';
import { pages, type PageName, type PageProps } from '../pages/index.ts';
import { downloadMarkdown, homeMarkdown } from '../markdown.ts';
import { imageRoute, imageTypes, linkedImages } from '../docs/links.ts';
import {
	docPaths,
	docRoute,
	docTitle,
	githubUrl,
	markdownRoute,
	sections,
	type DocPath,
} from '../docs/pages.ts';
import {
	channels,
	downloadChannel,
	repo,
	siteDescription,
	siteName,
	siteUrl,
	tagline,
} from '../site.ts';

// What a page needs of its render, which a not-found page in dev has too.
type PageContext = Pick<OxContentCustomHostRenderContext, 'mode' | 'assets'>;

// The Japanese display font is preloaded only where a page marks Japanese text, so the rest of
// the site doesn't fetch it; its @font-face still loads lazily by unicode-range.
function assetHead(context: PageContext, japanese: boolean): string {
	const selfHostedAssets = japanese
		? context.assets.selfHosted
		: { ...context.assets.selfHosted, preloads: [] };
	return context.mode === 'serve'
		? context.assets.document({
				sharedStyles: ['/src/styles/site.css', '/src/styles/docs.css'],
				clientEntries: ['/src/client/index.ts'],
				selfHostedAssets,
			}).headHtml
		: context.assets.document({
				clientEntries: ['index.html'],
				crossorigin: true,
				selfHostedAssets,
			}).headHtml;
}

const escapeHtml = (value: string) =>
	value.replaceAll('&', '&amp;').replaceAll('"', '&quot;').replaceAll('<', '&lt;');

const json = (value: unknown) => JSON.stringify(value).replaceAll('<', '\\u003c');

type Page = {
	name: string;
	title: string;
	description: string;
	pathname: string;
	markdown: string;
	markdownPath?: string;
};

function renderPage<Name extends PageName>(
	context: OxContentCustomHostRenderContext,
	page: Page & { name: Name },
	props: PageProps[Name],
) {
	const component = pages[page.name] as Component<PageProps[Name]>;
	return renderDocument(
		context,
		page,
		render(component, { props }),
		`<script type="application/json" id="page-props">${json(props)}</script>`,
	);
}

// A page in the site's frame: `data` follows the body, for the client entry to read.
function renderDocument(
	context: PageContext,
	page: Page,
	rendered: { head: string; body: string },
	data: string,
) {
	const url = new URL(page.pathname, siteUrl).href;
	const homeUrl = new URL('/', siteUrl).href;
	const image = new URL('/og.png', siteUrl).href;
	const imageAlt = escapeHtml(`${siteName}: ${tagline}`);
	const structuredData = {
		'@context': 'https://schema.org',
		'@graph': [
			{ '@type': 'WebSite', '@id': `${siteUrl}/#website`, url: homeUrl, name: siteName },
			{
				'@type': 'SoftwareApplication',
				'@id': `${siteUrl}/#app`,
				name: siteName,
				url: homeUrl,
				description: siteDescription,
				image,
				operatingSystem: 'macOS 14 or later (Apple silicon)',
				applicationCategory: 'UtilitiesApplication',
				downloadUrl: channels[downloadChannel].dmg,
				license: `${repo}/blob/main/LICENSE`,
			},
			{
				'@type': 'WebPage',
				'@id': `${url}#page`,
				url,
				name: page.title,
				description: page.description,
				isPartOf: { '@id': `${siteUrl}/#website` },
				about: { '@id': `${siteUrl}/#app` },
			},
		],
	};
	const html = `<!doctype html>
<html lang="en" data-theme="dark">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(page.title)}</title>
<meta name="description" content="${escapeHtml(page.description)}">
<meta name="theme-color" content="#000000">
<meta property="og:type" content="website">
<meta property="og:title" content="${escapeHtml(page.title)}">
<meta property="og:description" content="${escapeHtml(page.description)}">
<meta property="og:site_name" content="${siteName}">
<meta property="og:url" content="${url}">
<meta property="og:image" content="${image}">
<meta property="og:image:type" content="image/png">
<meta property="og:image:width" content="1200">
<meta property="og:image:height" content="630">
<meta property="og:image:alt" content="${imageAlt}">
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="${escapeHtml(page.title)}">
<meta name="twitter:description" content="${escapeHtml(page.description)}">
<meta name="twitter:image" content="${image}">
<meta name="twitter:image:alt" content="${imageAlt}">
<link rel="canonical" href="${url}">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="alternate" type="text/markdown" href="${page.markdownPath ?? (page.pathname === '/' ? '/index.md' : `${page.pathname.replace(/\/$/, '')}.md`)}">
<link rel="alternate" type="text/plain" href="/llms.txt" title="llms.txt">
<script type="application/ld+json">${json(structuredData)}</script>
${assetHead(context, page.markdown.includes('lang="ja"'))}
${rendered.head}
</head>
<body>
<div id="app" data-page="${page.name}">${rendered.body}</div>
${data}
</body>
</html>`;
	return { html, title: page.title, description: page.description, source: page.markdown };
}

const docsDir = (root: string) => path.join(root, '../docs');

// The first paragraph after the title, as plain text, for the page's description.
function docDescription(source: string): string {
	const paragraph = source
		.replace(/^# .+$/m, '')
		.split(/\n\s*\n/)
		.map((block) => block.trim())
		.find((block) => block && !/^[#<|`>-]/.test(block));
	return (paragraph ?? '')
		.replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
		.replace(/[`*_]/g, '')
		.replace(/\s+/g, ' ')
		.slice(0, 200);
}

const docsNav = (titles: ReadonlyMap<DocPath, string>) =>
	sections.map((section) => ({
		title: section.title,
		intro: 'intro' in section ? section.intro : undefined,
		pages: section.pages.map((page) => ({ title: titles.get(page) ?? page, href: docRoute(page) })),
	}));

// Search hits name a page by its path in docs/ without `.md`; only published ones have a route.
const searchPages = Object.fromEntries(
	docPaths.map((page) => [page.replace(/\.md$/, ''), docRoute(page)]),
);

async function docsRoutes(root: string): Promise<OxContentCustomHostRoute[]> {
	const sources = new Map(
		await Promise.all(
			docPaths.map(
				async (page) => [page, await readFile(path.join(docsDir(root), page), 'utf8')] as const,
			),
		),
	);
	const titles = new Map(docPaths.map((page) => [page, docTitle(sources.get(page) ?? '')]));
	const nav = docsNav(titles);
	const pagesRoutes = docPaths.map((page): OxContentCustomHostRoute => {
		const source = sources.get(page) ?? '';
		const inputPath = path.join('../docs', page);
		return {
			path: docRoute(page),
			inputPath,
			source,
			async render(context) {
				const { html } = await context.markdown.render({ source, documentPath: inputPath });
				const title = titles.get(page) ?? siteName;
				return {
					...renderDocument(
						context,
						{
							name: 'docs',
							title: `${title} · ${siteName} docs`,
							description: docDescription(source),
							pathname: docRoute(page),
							markdown: source,
							markdownPath: markdownRoute(page),
						},
						render(Docs, {
							props: {
								title,
								html,
								current: docRoute(page),
								sourceUrl: githubUrl(`docs/${page}`),
								sections: nav,
							},
						}),
						`<script type="application/json" id="docs-pages">${json(searchPages)}</script>`,
					),
					lastUpdatedPaths: [inputPath],
				};
			},
		};
	});
	// Every image in docs/images, and any other image of the repository a published page links to.
	const repoRoot = path.join(root, '..');
	const docsImages = (await readdir(path.join(docsDir(root), 'images'), { recursive: true })).map(
		(file) => `docs/images/${file.split(path.sep).join('/')}`,
	);
	const linked = docPaths.flatMap((page) =>
		linkedImages(repoRoot, path.join(docsDir(root), page), sources.get(page) ?? ''),
	);
	const imageRoutes = [...new Set([...docsImages, ...linked])].flatMap(
		(repoPath): OxContentCustomHostRoute[] => {
			const contentType = imageTypes[path.extname(repoPath).toLowerCase()];
			if (!contentType) return [];
			return [
				{
					path: imageRoute(repoPath),
					async render() {
						return { body: await readFile(path.join(repoRoot, repoPath)), contentType };
					},
				},
			];
		},
	);
	return [...pagesRoutes, ...imageRoutes];
}

const llmsDocs = (titles: (page: DocPath) => string) =>
	`\n## Documentation\n\n${docPaths
		.map((page) => `- [${titles(page)}](${siteUrl}${markdownRoute(page)})`)
		.join('\n')}\n`;

function notFound(context: PageContext) {
	const { source: _, ...page } = renderDocument(
		context,
		{
			name: 'not-found',
			title: `Page not found · ${siteName}`,
			description: siteDescription,
			pathname: '/404.html',
			markdown: '# Page not found\n',
		},
		render(NotFound),
		'',
	);
	return page;
}

const staticRoutes: OxContentCustomHostRoute[] = [
	{
		path: '/',
		render(context) {
			return {
				...renderPage(
					context,
					{
						name: 'home',
						title: `${siteName}: ${tagline}`,
						description: siteDescription,
						pathname: '/',
						markdown: homeMarkdown,
					},
					{},
				),
				lastUpdatedPaths: ['src/pages/Home.svelte', 'src/site.ts'],
			};
		},
	},
	{
		path: '/download',
		render(context) {
			return {
				...renderPage(
					context,
					{
						name: 'download',
						title: `Download ${siteName}`,
						description:
							'Download Dupin for Apple silicon Macs: the app, Homebrew casks, or a Nix build.',
						pathname: '/download/',
						markdown: downloadMarkdown,
					},
					{},
				),
				lastUpdatedPaths: ['src/pages/Download.svelte', 'src/site.ts', '../VERSION'],
			};
		},
	},
	{
		path: '/favicon.svg',
		async render(context) {
			return {
				body: await readFile(path.join(context.root, '../assets/logo/dupin.svg')),
				contentType: 'image/svg+xml',
			};
		},
	},
	{
		// Cloudflare serves it for any address the site has no file for.
		path: '/404.html',
		render: (context) => ({ ...notFound(context), status: 404 }),
	},
];

const host = {
	async routes(context) {
		const docs = await docsRoutes(context.root);
		const titles = new Map(
			docs
				.filter((route) => route.source)
				.map((route) => [route.path, docTitle(route.source ?? '')]),
		);
		return [
			...staticRoutes,
			{
				path: '/llms.txt',
				render() {
					return {
						body: homeMarkdown + llmsDocs((page) => titles.get(docRoute(page)) ?? page),
						contentType: 'text/plain; charset=utf-8',
					};
				},
			},
			...docs,
		];
	},
	notFound: (context) => ({ ...notFound(context), status: 404 }),
} satisfies OxContentCustomHostModule;

export default host;
