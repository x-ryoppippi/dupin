import { raw } from '@ox-content/vite-plugin';
import { readFileSync } from 'node:fs';

// The mark is square with the glasses across its middle; crop the viewBox to the glasses for the nav.
const mark = readFileSync(new URL('../../../assets/logo/dupin-mark.svg', import.meta.url), 'utf8')
	.replace(/ width="\d+" height="\d+"/, '')
	.replace(/viewBox="[^"]*"/, 'viewBox="100 350 870 415" class="logo" aria-hidden="true"');

export function Logo() {
	return raw(mark);
}

export function GitHubIcon() {
	return (
		<svg class="gh" viewBox="0 0 16 16" aria-hidden="true">
			<path
				fill="currentColor"
				d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"
			/>
		</svg>
	);
}

export function SearchIcon() {
	return (
		<svg class="field-icon" viewBox="0 0 16 16">
			<circle cx="7" cy="7" r="4.5" />
			<path d="M10.5 10.5L14 14" />
		</svg>
	);
}

export function CopyIcon() {
	return (
		<svg viewBox="0 0 16 16" aria-hidden="true">
			<rect x="5" y="5" width="8.5" height="8.5" rx="2" />
			<path d="M10.5 3.2A1.7 1.7 0 0 0 9 2.5H4.2a1.7 1.7 0 0 0-1.7 1.7V9a1.7 1.7 0 0 0 .7 1.5" />
		</svg>
	);
}
