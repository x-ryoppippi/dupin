import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { Renderer } from '@takumi-rs/core';
import { siteUrl } from '../site.ts';

export const ogWidth = 1200;
export const ogHeight = 630;

export type OgCard = {
	// The large line: the wordmark on the home page, the page title elsewhere.
	title: string;
	// The smaller line: the tagline on the home page, the docs section elsewhere.
	subtitle?: string;
	// Set on the home page, where the title is the wordmark and sits at its largest.
	wordmark?: boolean;
};

const escapeHtml = (value: string) =>
	value.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');

// The image's path on the site: the page's own path under /og/, so each card can be told apart
// and the file name never has to be guessed.
export function ogPath(pathname: string): string {
	const page = pathname.replace(/^\/|\/$|\.html$/g, '') || 'index';
	return `/og/${page}.png`;
}

// The card as markup in Takumi's CSS subset. The background and frame match the app's dark
// palette, and the logo is the one the favicon uses.
function markup({ title, subtitle, wordmark }: OgCard): string {
	const titleSize = wordmark ? 112 : title.length > 28 ? 72 : 88;
	return `<div style="display:flex;flex-direction:column;justify-content:space-between;width:${ogWidth}px;height:${ogHeight}px;padding:64px 68px 48px;background:radial-gradient(circle at 85% 10%, rgba(82,121,96,0.45), rgba(17,19,24,0) 90%), #111318;color:#f4f1ea;font-family:Inter">
<div style="position:absolute;left:32px;top:32px;width:1136px;height:566px;border:1px solid rgba(244,241,234,0.13);border-radius:40px"></div>
<img src="logo.svg" width="160" height="160">
<div style="display:flex;flex-direction:column;padding-left:20px">
<div style="font-size:${titleSize}px;font-weight:700;letter-spacing:${wordmark ? -5 : -3}px;line-height:1.1;max-width:1000px">${escapeHtml(title)}</div>
${subtitle ? `<div style="margin-top:${wordmark ? 20 : 24}px;font-size:${wordmark ? 43 : 36}px;font-weight:450;opacity:${wordmark ? 1 : 0.65};max-width:1000px">${escapeHtml(subtitle)}</div>` : ''}
</div>
<div style="display:flex;justify-content:flex-end;padding-right:24px;font-size:27px;opacity:0.65">dupin.app</div>
</div>`;
}

const hostDir = import.meta.dirname;

// The font and logo are read once and shared by every card.
const resources = (async () => {
	const [font, logo] = await Promise.all([
		readFile(path.join(hostDir, 'fonts/InterVariable.ttf')),
		readFile(path.join(hostDir, '../../../assets/logo/dupin.svg')),
	]);
	const renderer = new Renderer();
	await renderer.registerFont({ name: 'Inter', data: font });
	return { renderer, logo };
})();

async function renderOgCard(card: OgCard): Promise<Buffer> {
	const { renderer, logo } = await resources;
	return renderer.render(markup(card), {
		width: ogWidth,
		height: ogHeight,
		format: 'png',
		images: [{ src: 'logo.svg', data: logo }],
	});
}

export type OgImage = { path: string; url: string; png: Buffer };

const images = new Map<string, Promise<OgImage>>();

// Link previews are cached by address, so the address carries a hash of the card's pixels: a
// changed card is a new address, and an unchanged one keeps the old. A page and the route that
// serves its image share one render.
export function ogImage(pathname: string, card: OgCard): Promise<OgImage> {
	const key = JSON.stringify([pathname, card]);
	let image = images.get(key);
	if (!image) {
		image = renderOgCard(card).then((png) => {
			const imagePath = ogPath(pathname);
			const hash = createHash('sha256').update(png).digest('hex').slice(0, 8);
			return { path: imagePath, url: `${new URL(imagePath, siteUrl).href}?v=${hash}`, png };
		});
		images.set(key, image);
	}
	return image;
}
