import type { Child } from './types.ts';

export function Kbd({ children }: { children: Child }) {
	return <kbd class="kbd">{children}</kbd>;
}
