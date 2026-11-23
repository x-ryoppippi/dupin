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
		<span class={['copy-icon', state === 'copied' ? 'icon-[ph--check]' : 'icon-[ph--copy]']} aria-hidden="true"></span>
	{/if}
</button>
<span class="visually-hidden" role="status">
	{#if state === 'copied'}{copiedMessage}{:else if state === 'failed'}Could not copy. Select the text to copy it instead.{/if}
</span>

<style>
	.copy-button {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 36px;
		height: 36px;
		padding: 0;
		border-radius: var(--radius-sm);
		border: 1px solid var(--border-strong);
		color: var(--fg-muted);
		background: transparent;
		cursor: pointer;
		transition:
			color 0.18s ease,
			background 0.18s ease,
			border-color 0.18s ease;
	}

	.copy-button:hover {
		color: var(--fg);
		background: rgba(255, 255, 255, 0.04);
	}

	.copy-button.is-copied {
		color: var(--ok);
		border-color: color-mix(in srgb, var(--ok) 40%, transparent);
	}

	.copy-icon {
		width: 15px;
		height: 15px;
	}
</style>
