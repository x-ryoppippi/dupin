import type { Child } from './types.ts';

export function Check({ children }: { children: Child }) {
	return (
		<li>
			<svg class="check" viewBox="0 0 16 16" aria-hidden="true">
				<path d="M3.5 8.5l3 3 6-7" />
			</svg>
			<span>{children}</span>
		</li>
	);
}
