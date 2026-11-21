<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		text,
		label,
		copiedMessage,
		class: className = 'copy-button',
		children,
		copiedChildren,
	}: {
		text: string;
		label: string;
		copiedMessage: string;
		class?: string;
		children?: Snippet;
		copiedChildren?: Snippet;
	} = $props();

	let state = $state<'idle' | 'copied' | 'failed'>('idle');
	let timer: ReturnType<typeof setTimeout> | undefined;

	async function copy() {
		clearTimeout(timer);
		try {
			await navigator.clipboard.writeText(text);
			state = 'copied';
		} catch {
			state = 'failed';
		}
		timer = setTimeout(() => (state = 'idle'), 2000);
	}
</script>

<button class={className} class:is-copied={state === 'copied'} type="button" aria-label={label} onclick={copy}>
	{#if state === 'copied' && copiedChildren}
		{@render copiedChildren()}
	{:else if children}
		{@render children()}
	{:else}
		<svg class="copy-icon" viewBox="0 0 16 16" aria-hidden="true">
			{#if state === 'copied'}
				<path d="M3.5 8.5l3 3 6-7" />
			{:else}
				<rect x="5" y="5" width="8.5" height="8.5" rx="2" />
				<path d="M10.5 3.2A1.7 1.7 0 0 0 9 2.5H4.2a1.7 1.7 0 0 0-1.7 1.7V9a1.7 1.7 0 0 0 .7 1.5" />
			{/if}
		</svg>
	{/if}
</button>
<span class="visually-hidden" role="status">
	{#if state === 'copied'}{copiedMessage}{:else if state === 'failed'}Could not copy. Select the text to copy it instead.{/if}
</span>
