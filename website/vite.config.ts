import { cloudflare } from '@cloudflare/vite-plugin';
import { oxContentCustomHost, type OxContentOptions } from '@ox-content/vite-plugin';
import { svelte } from '@rsvelte/vite-plugin-svelte';
import { cloudflareRedirect } from '@ryoppippi/vite-plugin-cloudflare-redirect';
import { defineConfig } from 'vite';
import { repo, siteName, siteUrl } from './src/site.ts';

const oxContentOptions = {
	srcDir: 'content',
	highlight: true,
	search: false,
	docs: false,
	icons: { include: ['src/**/*.{css,ts,svelte}'] },
	ssg: {
		siteName,
		siteUrl,
		minifyHtml: true,
		readerChrome: { copy: true, externalLinks: false, backToTop: false },
	},
} satisfies OxContentOptions;

export default defineConfig({
	appType: 'mpa',
	plugins: [
		cloudflare(),
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
		}),
	],
	build: { rollupOptions: { input: 'index.html' } },
	// The site reads the logo and VERSION from the repository root, one level up.
	server: { fs: { allow: ['..'] } },
});
