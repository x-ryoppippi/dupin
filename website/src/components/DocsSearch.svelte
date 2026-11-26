<script lang="ts">
	import { search } from 'virtual:ox-content/search';

	type Props = { pages: Record<string, string> };
	type Hit = { title: string; href: string; snippet: string };

	let { pages }: Props = $props();
	let query = $state('');
	let hits = $state<Hit[]>([]);
	let latest = 0;

	// ox-content indexes every page in docs/ at its path without `.md`; only the published ones,
	// which `pages` maps to their routes, are shown.
	async function find(value: string) {
		const asked = ++latest;
		const results = value.trim() ? await search(value, { limit: 40 }) : [];
		if (asked !== latest) return;
		hits = results
			.map((result) => ({
				title: result.title,
				href: pages[result.url.replace(/^\//, '')],
				snippet: result.snippet,
			}))
			.filter((hit): hit is Hit => hit.href !== undefined)
			.slice(0, 8);
	}
</script>

<div class="search" role="search">
	<input
		type="search"
		placeholder="Search the docs"
		aria-label="Search the docs"
		bind:value={query}
		oninput={() => find(query)}
	/>
	{#if query.trim()}
		<ul class="hits">
			{#each hits as hit (hit.href)}
				<li>
					<a href={hit.href}>
						<span class="hit-title">{hit.title}</span>
						<span class="hit-snippet">{hit.snippet}</span>
					</a>
				</li>
			{:else}
				<li class="hit-none">No pages match.</li>
			{/each}
		</ul>
	{/if}
</div>

<style>
	.search {
		position: relative;
	}

	input {
		width: 100%;
		padding: 8px 10px;
		border-radius: var(--radius-sm);
		border: 1px solid var(--border-strong);
		background: var(--bg-card);
		color: var(--fg);
		font: inherit;
	}

	.hits {
		list-style: none;
		margin: 6px 0 0;
		padding: 4px;
		border-radius: var(--radius-sm);
		border: 1px solid var(--border-strong);
		background: var(--bg-elev);
	}

	.hits a {
		display: block;
		padding: 6px 8px;
		border-radius: 6px;
	}

	.hits a:hover,
	.hits a:focus-visible {
		background: var(--bg-card);
	}

	.hit-title {
		display: block;
		color: var(--fg);
	}

	.hit-snippet {
		display: block;
		font-size: 12px;
		color: var(--fg-dim);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.hit-none {
		padding: 6px 8px;
		color: var(--fg-dim);
	}
</style>
