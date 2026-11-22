import { readFile } from 'node:fs/promises';
import path from 'node:path';
import type {
	OxContentCustomHostModule,
	OxContentCustomHostRenderContext,
} from '@ox-content/vite-plugin/custom-host';
import { render } from 'svelte/server';
import type { Component } from 'svelte';
import { pages, type PageName, type PageProps } from '../pages/index.ts';
import { downloadMarkdown, homeMarkdown } from '../markdown.ts';
import {
	channels,
	downloadChannel,
	repo,
	siteDescription,
	siteName,
	siteUrl,
	tagline,
} from '../site.ts';

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
	page: { name: Name; title: string; description: string; pathname: string; markdown: string },
	props: PageProps[Name],
) {
	const component = pages[page.name] as Component<PageProps[Name]>;
	const rendered = render(component, { props });
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
<link rel="alternate" type="text/markdown" href="${page.pathname === '/' ? '/index.md' : `${page.pathname.replace(/\/$/, '')}.md`}">
<link rel="alternate" type="text/plain" href="/llms.txt" title="llms.txt">
<script type="application/ld+json">${JSON.stringify(structuredData).replaceAll('<', '\\u003c')}</script>
${assetHead(context)}
${rendered.head}
</head>
<body>
<div id="app" data-page="${page.name}">${rendered.body}</div>
<script type="application/json" id="page-props">${JSON.stringify(props).replaceAll('<', '\\u003c')}</script>
</body>
</html>`;
	return { html, title: page.title, description: page.description, source: page.markdown };
}

const host = {
	routes: [
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
							title: `Download ${siteName}, ${tagline}`,
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
			path: '/llms.txt',
			render() {
				return { body: homeMarkdown, contentType: 'text/plain; charset=utf-8' };
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
