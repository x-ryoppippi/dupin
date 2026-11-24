import { readFileSync, writeFileSync } from 'node:fs';
import { builtinModules } from 'node:module';
import { relative } from 'node:path';
import type { RolldownOptions } from 'rolldown';

// `dupin ext build`, `dupin ext dev` and `.github/workflows/build.yml` all bundle with this
// file. Dupin runs `dupin.main` as one ES module with nothing to import but `dupin`.
const pkg = JSON.parse(readFileSync('package.json', 'utf8'));
const builtins = new Set(builtinModules);

export default {
	input: pkg.source,
	platform: 'neutral',
	external: ['dupin'],
	resolve: { mainFields: ['module', 'main'] },
	plugins: [
		{
			// The guest has no Node built-ins; failing here names the file that wanted one.
			name: 'dupin:no-node-builtins',
			resolveId(id, importer) {
				if (!id.startsWith('node:') && !builtins.has(id)) return null;
				const file = importer ? relative(process.cwd(), importer) : pkg.source;
				const report = process.env.DUPIN_BUILD_REPORT;
				if (report)
					writeFileSync(report, JSON.stringify({ nodeBuiltin: { importer: file, specifier: id } }));
				return this.error(
					`${file} imports the Node built-in \`${id}\`, which Dupin extensions don't have`,
				);
			},
		},
	],
	onLog(level, log, handler) {
		// Rolldown keeps an import it can't resolve as an external one, which the guest can't load.
		handler(log.code === 'UNRESOLVED_IMPORT' ? 'error' : level, log);
	},
	output: {
		file: pkg.dupin?.main ?? 'main.js',
		format: 'esm',
		codeSplitting: false,
	},
} satisfies RolldownOptions;
