import { readFile } from 'node:fs/promises';
import path from 'node:path';
import type {
	OxContentCustomHostModule,
	OxContentCustomHostRenderContext,
} from '@ox-content/vite-plugin/custom-host';
import { renderThemeTokenCss } from '@ox-content/vite-plugin';
import {
	renderReaderChromeScriptTag,
	renderReaderChromeStyleTag,
} from '@ox-content/vite-plugin/reader-chrome';
import { vitesseBlack } from '@ox-content/theme-color-vitesse';
import { render } from 'svelte/server';
import type { Component } from 'svelte';
import { pages, type PageName, type PageProps } from '../pages/index.ts';
import { siteName, siteUrl } from '../site.ts';

const readerChrome = { copy: true, externalLinks: false, backToTop: false } as const;

// Only the code colors come from the palette; the page owns everything else.
const syntaxCss = renderThemeTokenCss(vitesseBlack, {
	include: (name) => name.startsWith('syntax-'),
});

function assetHead(context: OxContentCustomHostRenderContext): string {
	return context.mode === 'serve'
		? context.assets.document({
				sharedStyles: ['/src/styles/site.css'],
				clientEntries: ['/src/client/index.ts'],
				selfHostedAssets: context.assets.selfHosted,
			}).headHtml
		: context.assets.document({
				clientEntries: ['index.html'],
				crossorigin: true,
				selfHostedAssets: context.assets.selfHosted,
			}).headHtml;
}

const escapeHtml = (value: string) =>
	value.replaceAll('&', '&amp;').replaceAll('"', '&quot;').replaceAll('<', '&lt;');

function renderPage<Name extends PageName>(
	context: OxContentCustomHostRenderContext,
	page: { name: Name; title: string; description: string; pathname: string },
	props: PageProps[Name],
) {
	const component = pages[page.name] as Component<PageProps[Name]>;
	const rendered = render(component, { props });
	const url = new URL(page.pathname, siteUrl).href;
	const html = `<!doctype html>
<html lang="en" data-theme="dark">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(page.title)}</title>
<meta name="description" content="${escapeHtml(page.description)}">
<meta name="theme-color" content="#000000">
<meta property="og:title" content="${escapeHtml(page.title)}">
<meta property="og:description" content="${escapeHtml(page.description)}">
<meta property="og:site_name" content="${siteName}">
<meta property="og:url" content="${url}">
<link rel="canonical" href="${url}">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<style>${syntaxCss}</style>
${renderReaderChromeStyleTag(readerChrome)}
${assetHead(context)}
${rendered.head}
</head>
<body>
<div id="app" data-page="${page.name}">${rendered.body}</div>
<script type="application/json" id="page-props">${JSON.stringify(props).replaceAll('<', '\\u003c')}</script>
${renderReaderChromeScriptTag(readerChrome)}
</body>
</html>`;
	return { html, title: page.title, description: page.description };
}

const description =
	'A lightweight, resident launcher for Apple silicon Macs. Press ⌘Space and the panel is already there.';

const host = {
	routes: [
		{
			path: '/',
			inputPath: 'content/index.md',
			async render(context) {
				const documentPath = 'content/index.md';
				const source = await readFile(path.join(context.root, documentPath), 'utf8');
				const install = await context.markdown.render({ source, documentPath });
				return {
					...renderPage(
						context,
						{
							name: 'home',
							title: `${siteName}: a launcher for Apple silicon Macs`,
							description,
							pathname: '/',
						},
						{ installHtml: install.html },
					),
					dependencies: install.dependencies,
				};
			},
		},
		{
			path: '/download',
			render(context) {
				return renderPage(
					context,
					{
						name: 'download',
						title: `Download ${siteName}`,
						description:
							'Download Dupin for Apple silicon Macs: the app, Homebrew casks, or a Nix build.',
						pathname: '/download',
					},
					{},
				);
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
	],
} satisfies OxContentCustomHostModule;

export default host;
