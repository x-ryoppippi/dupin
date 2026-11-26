import { defineConfig } from 'cf/config';

export default defineConfig({
	accountId: 'a2b8b26fa722742bc36a159fc3b009fb',
	worker: {
		name: 'dupin',
		compatibilityDate: '2026-11-21',
		domains: ['dupin.app'],
		previewUrls: true,
		assets: {
			htmlHandling: 'auto-trailing-slash',
			notFoundHandling: '404-page',
		},
	},
});
