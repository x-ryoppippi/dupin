import { raw, renderThemeTokenCss, usePageProps, useSiteConfig } from '@ox-content/vite-plugin';
import { vitesseBlack } from '@ox-content/theme-color-vitesse';
import { siteUrl } from '../site.ts';

// Only the code colors come from the palette; the page owns everything else.
const syntaxCss = renderThemeTokenCss(vitesseBlack, {
	include: (name) => name.startsWith('syntax-'),
});

export function Head() {
	const page = usePageProps();
	const site = useSiteConfig();
	const description = page.description ?? '';
	return (
		<head>
			<meta charset="utf-8" />
			<meta name="viewport" content="width=device-width, initial-scale=1" />
			<title>{`${site.name}: a launcher for Apple silicon Macs`}</title>
			<meta name="description" content={description} />
			<meta name="theme-color" content="#000000" />
			<meta property="og:title" content={site.name} />
			<meta property="og:description" content={description} />
			<meta property="og:url" content={siteUrl} />
			<link rel="canonical" href={siteUrl} />
			<link rel="icon" href="/favicon.svg" type="image/svg+xml" />
			<link rel="stylesheet" href="/site.css" />
			<style>{raw(syntaxCss)}</style>
			<script type="module" src="/motion.js" />
		</head>
	);
}
