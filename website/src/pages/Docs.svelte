<script lang="ts">
	import Footer from '../components/Footer.svelte';
	import Nav from '../components/Nav.svelte';

	type Props = {
		title: string;
		html: string;
		current: string;
		sourceUrl: string;
		sections: { title: string; pages: { title: string; href: string }[] }[];
	};

	let { title, html, current, sourceUrl, sections }: Props = $props();
</script>

<Nav />
<div class="docs">
	<aside class="docs-sidebar" aria-label="Documentation">
		<div id="docs-search" class="docs-search"></div>
		<nav>
			{#each sections as section (section.title)}
				<p class="docs-section">{section.title}</p>
				<ul>
					{#each section.pages as page (page.href)}
						<li>
							<a href={page.href} aria-current={page.href === current ? 'page' : undefined}
								>{page.title}</a
							>
						</li>
					{/each}
				</ul>
			{/each}
		</nav>
	</aside>
	<main class="docs-main">
		<article class="docs-content" aria-label={title}>
			{@html html}
		</article>
		<p class="docs-source"><a href={sourceUrl}>View this page on GitHub</a></p>
	</main>
</div>
<Footer />

<style>
	.docs {
		max-width: 1240px;
		margin: 0 auto;
		padding: 32px;
		display: grid;
		grid-template-columns: 240px minmax(0, 1fr);
		gap: 48px;
	}

	.docs-sidebar {
		position: sticky;
		top: 72px;
		align-self: start;
		max-height: calc(100vh - 96px);
		overflow-y: auto;
		font-size: 14px;
	}

	.docs-section {
		margin: 20px 0 6px;
		font-size: 12px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.04em;
		color: var(--fg-dim);
	}

	.docs-sidebar ul {
		list-style: none;
		margin: 0;
		padding: 0;
	}

	.docs-sidebar a {
		display: block;
		padding: 4px 8px;
		border-radius: var(--radius-sm);
		color: var(--fg-muted);
	}

	.docs-sidebar a:hover,
	.docs-sidebar a[aria-current='page'] {
		color: var(--fg);
		background: var(--bg-card);
	}

	.docs-main {
		min-width: 0;
		max-width: 780px;
	}

	.docs-source {
		margin-top: 48px;
		font-size: 13px;
		color: var(--fg-dim);
	}

	.docs-content {
		line-height: 1.65;
		color: var(--fg-muted);
	}

	.docs-content :global(h1),
	.docs-content :global(h2),
	.docs-content :global(h3),
	.docs-content :global(h4) {
		color: var(--fg);
		line-height: 1.25;
		letter-spacing: -0.01em;
		scroll-margin-top: 80px;
	}

	.docs-content :global(h1) {
		font-size: 36px;
		margin: 0 0 24px;
	}

	.docs-content :global(h2) {
		font-size: 24px;
		margin: 48px 0 16px;
	}

	.docs-content :global(h3) {
		font-size: 18px;
		margin: 32px 0 12px;
	}

	.docs-content :global(a) {
		color: var(--accent);
	}

	.docs-content :global(img) {
		max-width: 100%;
		height: auto;
		border-radius: var(--radius);
	}

	.docs-content :global(code) {
		font-family: var(--font-mono);
		font-size: 0.9em;
		padding: 0.1em 0.35em;
		border-radius: 4px;
		background: var(--bg-card);
		border: 1px solid var(--border);
	}

	.docs-content :global(pre) {
		overflow-x: auto;
		padding: 16px;
		border-radius: var(--radius);
		background: var(--bg-card);
		border: 1px solid var(--border);
	}

	.docs-content :global(pre code) {
		padding: 0;
		border: 0;
		background: none;
	}

	.docs-content :global(table) {
		display: block;
		overflow-x: auto;
		border-collapse: collapse;
		font-size: 14px;
	}

	.docs-content :global(th),
	.docs-content :global(td) {
		padding: 6px 12px;
		border: 1px solid var(--border);
		text-align: left;
		vertical-align: top;
	}

	@media (max-width: 820px) {
		.docs {
			grid-template-columns: 1fr;
			padding: 16px;
			gap: 16px;
		}

		.docs-sidebar {
			position: static;
			max-height: none;
		}
	}
</style>
