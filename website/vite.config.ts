import { cloudflare } from '@cloudflare/vite-plugin';
import { oxContentCustomHost, type OxContentOptions } from '@ox-content/vite-plugin';
import { svelte } from '@rsvelte/vite-plugin-svelte';
import { cloudflareRedirect } from '@ryoppippi/vite-plugin-cloudflare-redirect';
import { defineConfig, type Plugin } from 'vite';
import { repo, siteName, siteUrl } from './src/site.ts';

const oxContentOptions = {
	search: false,
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
