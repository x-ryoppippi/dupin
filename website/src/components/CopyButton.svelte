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
