<script lang="ts">
	import Footer from '../components/Footer.svelte';
	import Nav from '../components/Nav.svelte';
</script>

<Nav />
<main class="not-found">
	<h1 class="display">Page not found</h1>
	<p>There is nothing at this address. Try the <a href="/docs/">documentation</a> or the <a href="/">home page</a>.</p>
</main>
<Footer />

<style>
	.not-found {
		max-width: 720px;
		margin: 0 auto;
		padding: 120px 32px;
		text-align: center;
		color: var(--fg-muted);
	}

	.not-found a {
		color: var(--accent);
	}
</style>
