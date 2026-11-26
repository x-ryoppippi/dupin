import { cloudflare } from '@cloudflare/vite-plugin';
import { oxContentCustomHost, type OxContentOptions } from '@ox-content/vite-plugin';
import { svelte } from '@rsvelte/vite-plugin-svelte';
import { cloudflareRedirect } from '@ryoppippi/vite-plugin-cloudflare-redirect';
import path from 'node:path';
import { defineConfig, type Plugin } from 'vite';
import { docsLinks } from './src/docs/links.ts';
import { repo, siteName, siteUrl } from './src/site.ts';

const oxContentOptions = {
	// The documentation is the Markdown in the repository's docs/, rendered as it is on GitHub.
	srcDir: '../docs',
	transformers: [docsLinks(path.resolve(import.meta.dirname, '..'))],
	search: { limit: 40 },
	// Where `cf build` puts the Worker's static assets, which the search index is one of.
	outDir: '.cloudflare/output/v1/workers/default/assets',
	docs: false,
	icons: { include: ['src/**/*.{css,ts,svelte}'] },
	siteMaps: { llms: false },
	ssg: {
		siteName,
		siteUrl,
		minifyHtml: true,
		markdownSource: { alternate: true },
	},
} satisfies OxContentOptions;

const workerEnvironment = 'redirect';

export default defineConfig({
	appType: 'mpa',
	plugins: [
		cloudflare({ viteEnvironment: { name: workerEnvironment } }),
		svelte({ configFile: false }),
		cloudflareRedirect({
			mode: 'generate',
			entries: [
				{ from: '/gh', to: repo, status: 302 },
				{ from: '/gh/*', to: `${repo}/:splat`, status: 302 },
			],
		}),
		...oxContentCustomHost({
			host: '/src/host/index.ts',
			oxContent: oxContentOptions,
			dev: { routeDependencies: [{ path: 'src', kind: 'directory' }] },
			build: { transformHtml: false },
		}).map(
			(plugin) =>
				({
					...plugin,
					applyToEnvironment(environment) {
						return (
							environment.name !== workerEnvironment &&
							(plugin.applyToEnvironment?.(environment) ?? true)
						);
					},
				}) satisfies Plugin,
		),
	],
	build: { rollupOptions: { input: 'index.html' } },
	// The site reads the logo and VERSION from the repository root, one level up.
	server: { fs: { allow: ['..'] } },
});
