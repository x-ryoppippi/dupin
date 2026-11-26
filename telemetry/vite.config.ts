import { voidSvelte } from '@void/svelte/plugin';
import { voidPlugin } from 'void';
import { defineConfig } from 'vitest/config';

export default defineConfig({
	build: { assetsInlineLimit: 0, minify: true },
	define: { 'import.meta.vitest': 'undefined' },
	plugins: [voidPlugin(), voidSvelte()],
	// The dashboard uses the logo from the repository root, one level up.
	server: { fs: { allow: ['..'] } },
	test: {
		globals: true,
		include: [],
		includeSource: ['{src,crons}/**/*.ts'],
		env: { TZ: 'UTC' },
	},
});
