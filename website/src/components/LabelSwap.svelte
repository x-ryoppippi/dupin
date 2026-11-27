<script lang="ts">
	import type { Snippet } from 'svelte';

	let { labels, current }: { labels: { id: string; content: Snippet }[]; current: string } = $props();
</script>

<!-- Every label shares one grid cell, so a button sizes to its widest label whichever is showing. -->
<span class="label-swap">
	{#each labels as label (label.id)}
		<span class="label" data-label={label.id} aria-hidden={label.id === current ? undefined : 'true'}>
			{@render label.content()}
		</span>
	{/each}
</span>

<style>
	.label-swap {
		display: grid;
		gap: inherit;
	}

	.label {
		grid-area: 1 / 1;
		display: inline-flex;
		align-items: center;
		justify-content: center;
		gap: inherit;
		transition:
			opacity 0.15s ease,
			visibility 0.15s;
	}

	.label[aria-hidden='true'] {
		visibility: hidden;
		opacity: 0;
	}
</style>
