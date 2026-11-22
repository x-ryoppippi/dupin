import assert from 'node:assert/strict';
import test from 'node:test';
import worker from './worker.ts';

await test('canonical HTTP requests use 308 and preserve path/query without reading the body', async () => {
	const env = {
		ASSETS: {
			fetch: async () => {
				throw new Error('HTTP canonical requests must not reach assets');
			},
		},
	} satisfies Parameters<typeof worker.fetch>[1];
	for (const method of ['GET', 'HEAD', 'POST']) {
		for (const pathname of ['/', '/download/?from=issue94&next=%2Fnotes', '/missing']) {
			const request = new Request(`http://dupin.app${pathname}`, {
				method,
				...(method === 'POST' ? { body: 'fixture' } : {}),
			});
			const response = await worker.fetch(request, env);
			assert.equal(response.status, 308);
			assert.equal(response.headers.get('location'), `https://dupin.app${pathname}`);
			assert.equal(request.method, method);
			assert.equal(request.bodyUsed, false);
		}
	}
});

await test('HTTPS, local and preview requests retain the asset response and original request', async () => {
	for (const url of [
		'https://dupin.app/',
		'https://dupin.app/assets/site.css',
		'https://dupin.app/gh',
		'https://dupin.app/missing',
		'http://localhost:5173/download/',
		'http://127.0.0.1:5173/',
		'http://preview.example.workers.dev/',
	]) {
		for (const status of [200, 302, 404]) {
			const request = new Request(url);
			const asset = new Response('asset fixture', {
				status,
				headers: { 'x-fixture': 'preserved' },
			});
			const env = {
				ASSETS: {
					fetch: async (received: Request) => {
						assert.equal(received, request);
						return asset;
					},
				},
			} satisfies Parameters<typeof worker.fetch>[1];
			assert.equal(await worker.fetch(request, env), asset);
		}
	}
});
