<script lang="ts">
	import { onMount } from 'svelte';
	import { siteName, siteUrl } from '../site.ts';
	import LabelSwap from './LabelSwap.svelte';

	let { href }: { href: string } = $props();

	// A phone or tablet cannot install the Mac app, so there the button sends the site to a Mac
	// instead. The same query in the style block picks the label before hydration.
	const touchOnly = '(hover: none) and (pointer: coarse)';

	let mode = $state<'download' | 'share'>();
	let state = $state<'idle' | 'copied' | 'failed'>('idle');
	let timer: ReturnType<typeof setTimeout> | undefined;

	onMount(() => {
		const query = matchMedia(touchOnly);
		const update = () => (mode = query.matches ? 'share' : 'download');
		update();
		query.addEventListener('change', update);
		return () => query.removeEventListener('change', update);
	});

	async function send() {
		clearTimeout(timer);
		state = 'idle';
		if (navigator.share) {
			try {
				await navigator.share({
					title: siteName,
					text: 'Dupin, a launcher for Apple silicon Macs. Open this on your Mac to download it.',
					url: siteUrl,
				});
				return;
			} catch (error) {
				if (error instanceof DOMException && error.name === 'AbortError') return;
			}
		}
		try {
			await navigator.clipboard.writeText(siteUrl);
			state = 'copied';
		} catch {
			state = 'failed';
		}
		timer = setTimeout(() => (state = 'idle'), 2000);
	}

	const current = $derived(mode === 'share' ? (state === 'copied' ? 'copied' : 'share') : 'download');
</script>

{#snippet download()}
	<span class="btn-icon icon-[simple-icons--apple]" aria-hidden="true"></span>
	Download for macOS
{/snippet}

{#snippet share()}
	<span class="btn-icon icon-[ph--export]" aria-hidden="true"></span>
	Send to your Mac
{/snippet}

{#snippet copied()}
	<span class="btn-icon icon-[ph--check]" aria-hidden="true"></span>
	Link copied
{/snippet}

{#snippet labels()}
	<LabelSwap
		labels={[
			{ id: 'download', content: download },
			{ id: 'share', content: share },
			{ id: 'copied', content: copied },
		]}
		{current}
	/>
{/snippet}

{#if mode === 'share'}
	<button class="btn-primary" type="button" data-mode={mode} onclick={send}>{@render labels()}</button>
	<span class="visually-hidden" role="status">
		{#if state === 'copied'}Copied the link to Dupin. Open it on your Mac.{:else if state === 'failed'}Could not share or copy the link. Open {siteUrl} on your Mac.{/if}
	</span>
{:else}
	<a class="btn-primary" {href} data-mode={mode}>{@render labels()}</a>
{/if}

<style>
	@media (hover: none) and (pointer: coarse) {
		.btn-primary:not([data-mode]) :global([data-label='download']) {
			visibility: hidden;
		}

		.btn-primary:not([data-mode]) :global([data-label='share']) {
			visibility: visible;
			opacity: 1;
		}
	}
</style>
