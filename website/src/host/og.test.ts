import assert from 'node:assert/strict';
import test from 'node:test';
import { ogImage, ogPath } from './og.ts';

await test('a card lives at its page path under /og/', () => {
	assert.equal(ogPath('/'), '/og/index.png');
	assert.equal(ogPath('/download/'), '/og/download.png');
	assert.equal(ogPath('/docs/features/calculator/'), '/og/docs/features/calculator.png');
});

await test('a card is a 1200 x 630 PNG whose address carries a hash of its pixels', async () => {
	const first = await ogImage('/docs/a/', { title: 'First' });
	const same = await ogImage('/docs/a/', { title: 'First' });
	const other = await ogImage('/docs/b/', { title: 'Second' });

	assert.equal(first.png.subarray(1, 4).toString(), 'PNG');
	assert.equal(first.png.readUInt32BE(16), 1200);
	assert.equal(first.png.readUInt32BE(20), 630);
	assert.match(first.url, /^https:\/\/dupin\.app\/og\/docs\/a\.png\?v=[0-9a-f]{8}$/);
	assert.equal(same.url, first.url);
	assert.notEqual(other.url.split('?v=')[1], first.url.split('?v=')[1]);
});
