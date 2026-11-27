declare module 'virtual:ox-content/search' {
	import type { SearchResult } from '@ox-content/vite-plugin';

	export function search(query: string, options?: { limit?: number }): Promise<SearchResult[]>;
}
