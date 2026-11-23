<script lang="ts">
	import mark from '../../../assets/logo/dupin-mark.svg?raw';

	let { width = 34, height = 16 }: { width?: number; height?: number } = $props();

	// The mark is square with the glasses across its middle; the viewBox below crops it to the glasses.
	const paths = mark.replace(/^[\s\S]*?<svg[^>]*>/, '').replace(/<\/svg>\s*$/, '');
</script>

<svg class="logo" viewBox="100 350 870 415" {width} {height} aria-hidden="true">{@html paths}</svg>

<style>
	.logo {
		color: var(--fg);
	}
</style>
