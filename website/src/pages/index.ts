import type { Component } from 'svelte';
import Download from './Download.svelte';
import Home from './Home.svelte';

export type PageProps = {
	home: Record<string, never>;
	download: Record<string, never>;
};
export type PageName = keyof PageProps;

export const pages = { home: Home, download: Download } satisfies {
	[Name in PageName]: Component<PageProps[Name]>;
};
