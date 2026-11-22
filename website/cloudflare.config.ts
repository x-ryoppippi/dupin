import { bindings, defineConfig } from 'cf/config';

export default defineConfig({
	accountId: 'a2b8b26fa722742bc36a159fc3b009fb',
	worker: {
		name: 'dupin',
		entrypoint: './src/worker.ts',
		compatibilityDate: '2026-11-21',
		domains: ['dupin.app'],
		previewUrls: true,
		env: { ASSETS: bindings.assets() },
		assets: {
			runWorkerFirst: true,
			htmlHandling: 'auto-trailing-slash',
			notFoundHandling: '404-page',
		},
	},
});
