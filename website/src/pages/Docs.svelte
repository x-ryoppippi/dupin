<script lang="ts">
	import Footer from '../components/Footer.svelte';
	import Nav from '../components/Nav.svelte';

	type Props = {
		title: string;
		html: string;
		current: string;
		sourceUrl: string;
		sections: { title: string; intro?: string; pages: { title: string; href: string }[] }[];
	};

	let { title, html, current, sourceUrl, sections }: Props = $props();

	const currentTitle = $derived(
		sections.flatMap((section) => section.pages).find((page) => page.href === current)?.title ??
			title,
	);
</script>

<Nav />
<div class="docs">
	<aside class="docs-sidebar" aria-label="Documentation">
		<div id="docs-search" class="docs-search"></div>
		<details class="docs-toc">
			<summary><span class="docs-toc-label">Docs</span><span class="docs-toc-current">{currentTitle}</span></summary>
			<nav>
			{#each sections as section (section.title)}
				<p class="docs-section">{section.title}</p>
				{#if section.intro}
					<p class="docs-section-intro">{section.intro}</p>
				{/if}
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
		</details>
	</aside>
	<main class="docs-main">
		<article class="docs-content" aria-label={title}>
			{@html html}
		</article>
		<p class="docs-source"><a href={sourceUrl}>View this page on GitHub</a></p>
	</main>
</div>
<Footer />

