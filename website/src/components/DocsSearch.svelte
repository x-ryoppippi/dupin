<script lang="ts">
	import { onMount } from 'svelte';
	import { search } from 'virtual:ox-content/search';

	type Props = { pages: Record<string, string> };
	type Hit = { title: string; href: string; snippet: string };

	let { pages }: Props = $props();
	let query = $state('');
	let hits = $state<Hit[]>([]);
	let active = $state(-1);
	let input = $state<HTMLInputElement>();
	let latest = 0;
	// Decided on mount, as the server cannot know the visitor's platform.
	let hint = $state('');

	onMount(() => {
		hint = /Mac|iPhone|iPad/.test(navigator.platform) ? '⌘K' : 'Ctrl K';
	});

	// ox-content indexes every page in docs/ at its path without `.md`; only the published ones,
	// which `pages` maps to their routes, are shown.
	async function find(value: string) {
		const asked = ++latest;
		const results = value.trim() ? await search(value, { limit: 40 }) : [];
		if (asked !== latest) return;
		active = -1;
		hits = results
			.map((result) => ({
				title: result.title,
				href: pages[result.url.replace(/^\//, '')],
				snippet: result.snippet,
			}))
			.filter((hit): hit is Hit => hit.href !== undefined)
			.slice(0, 8);
	}

	function onWindowKeydown(event: KeyboardEvent) {
		if (event.key.toLowerCase() !== 'k' || event.altKey || event.shiftKey) return;
		if (!(event.metaKey || event.ctrlKey)) return;
		// Leave the shortcut alone in any other text field, where it may mean something else.
		const target = event.target;
		const elsewhere =
			target instanceof HTMLElement &&
			target !== input &&
			(target.isContentEditable || target.matches('input, textarea, select'));
		if (elsewhere) return;
		event.preventDefault();
		input?.focus();
		input?.select();
	}

	function onInputKeydown(event: KeyboardEvent) {
		if (event.key === 'Escape') {
			if (query) {
				query = '';
				hits = [];
				latest++;
			} else {
				input?.blur();
			}
		} else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
			if (!hits.length) return;
			event.preventDefault();
			const down = event.key === 'ArrowDown';
			active = down ? (active + 1) % hits.length : (active <= 0 ? hits.length : active) - 1;
		} else if (event.key === 'Enter') {
			const hit = hits[active === -1 ? 0 : active];
			if (hit && query.trim()) {
				event.preventDefault();
				location.href = hit.href;
			}
		}
	}
</script>

<svelte:window onkeydown={onWindowKeydown} />

<div class="search" role="search">
	<input
		bind:this={input}
		type="search"
		placeholder="Search the docs"
		aria-label="Search the docs"
		aria-controls="docs-hits"
		aria-activedescendant={active >= 0 ? `docs-hit-${active}` : undefined}
		bind:value={query}
		oninput={() => find(query)}
		onkeydown={onInputKeydown}
	/>
	{#if hint && !query}
		<kbd aria-hidden="true">{hint}</kbd>
	{/if}
	{#if query.trim()}
		<ul class="hits" id="docs-hits">
			{#each hits as hit, index (hit.href)}
				<li>
					<a id="docs-hit-{index}" href={hit.href} class:active={index === active}>
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
		box-sizing: border-box;
		padding: 8px 56px 8px 10px;
		border-radius: var(--radius-sm);
		border: 1px solid var(--border-strong);
		background: var(--bg-card);
		color: var(--fg);
		font: inherit;
	}

	kbd {
		position: absolute;
		top: 8px;
		right: 8px;
		padding: 1px 6px;
		border: 1px solid var(--border-strong);
		border-radius: 4px;
		font: inherit;
		font-size: 12px;
		color: var(--fg-dim);
		pointer-events: none;
	}

	@media (hover: none) {
		kbd {
			display: none;
		}
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

	.hits a.active {
		background: var(--border-strong);
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
