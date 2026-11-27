import assert from 'node:assert/strict';
import { readdirSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { linkedImages, siteUrl } from './links.ts';
import { isPublished } from './pages.ts';

const root = path.resolve(import.meta.dirname, '../../..');
const calendar = path.join(root, 'docs/features/calendar.md');

await test('links reach published pages on the site and everything else on GitHub', () => {
	assert.equal(
		siteUrl(root, calendar, './keywords.md#aliases'),
		'/docs/features/keywords/#aliases',
	);
	assert.equal(siteUrl(root, calendar, '../usage.md'), '/docs/');
	assert.equal(
		siteUrl(root, calendar, '../development.md#headless-renders'),
		'https://github.com/x-ryoppippi/dupin/blob/main/docs/development.md#headless-renders',
	);
	assert.equal(
		siteUrl(root, calendar, '../../extensions/calendar'),
		'https://github.com/x-ryoppippi/dupin/tree/main/extensions/calendar',
	);
	assert.equal(
		siteUrl(root, calendar, '../images/calendar/upcoming-dark.png'),
		'/docs/images/calendar/upcoming-dark.png',
	);
	assert.equal(
		siteUrl(root, calendar, '../../assets/logo/dupin.svg'),
		'/docs/files/assets/logo/dupin.svg',
	);
	assert.equal(siteUrl(root, calendar, '#configuration'), '#configuration');
	assert.equal(siteUrl(root, calendar, 'https://example.com/'), 'https://example.com/');
});

await test('images outside docs/images that a page links to are listed for the site to serve', () => {
	const name = path.join(root, 'docs/name.md');
	assert.deepEqual(
		linkedImages(
			root,
			name,
			'<img src="../assets/logo/dupin.svg" /> ![a](images/usage/search-dark.png) [b](./usage.md) ![c](<../assets/x.png>) <img src="https://example.com/a.png">',
		),
		['assets/logo/dupin.svg', 'assets/x.png'],
	);
});

await test('every feature page is published', () => {
	for (const file of readdirSync(path.join(root, 'docs/features'))) {
		assert.ok(isPublished(`features/${file}`), `docs/features/${file} is in no section`);
	}
});
