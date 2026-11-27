import { defineConfig } from 'void/config';

// The rollups' KV namespace, which deploying needs and nothing else does. Its ID is not a secret;
// the deploy workflow passes the repository variable of the same name (see README.md).
const rollupsNamespace = process.env.DUPIN_TELEMETRY_KV_NAMESPACE_ID || 'local';

export default defineConfig({
	worker: { compatibility_date: '2026-10-01' },
	inference: { bindings: { db: false, kv: false, storage: false, ai: false } },
	cloudflare: {
		name: 'dupin-telemetry',
		account_id: 'a2b8b26fa722742bc36a159fc3b009fb',
		routes: [{ pattern: 'telemetry.dupin.app', custom_domain: true }],
		workers_dev: false,
		preview_urls: false,
		cache: { enabled: true },
		observability: { enabled: false },
		analytics_engine_datasets: [{ binding: 'PINGS', dataset: 'dupin_telemetry' }],
		analytics: { binding: 'ANALYTICS' },
		kv_namespaces: [{ binding: 'ROLLUPS', id: rollupsNamespace }],
		version_metadata: { binding: 'VERSION' },
	},
});
