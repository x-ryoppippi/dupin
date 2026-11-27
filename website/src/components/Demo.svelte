<script lang="ts">
	import glass from '../../../docs/images/glass.json';
	import Clip from './Clip.svelte';

	// The home page's recordings from scripts/docs-images/images.toml: a tab per feature over
	// Liquid Glass. Only the one shown is in the page, so the rest load when picked.
	type Recording = { src: string; poster: string; label: string };

	const tabs: { id: string; title: string; key?: string; recording: Recording }[] = [
		{
			id: 'apps',
			title: 'Apps',
			recording: {
				src: '/docs/images/home/apps-dark.mp4',
				poster: '/docs/images/home/apps-dark-poster.webp',
				label: 'Typing maps lists Maps first; ⌘K opens its actions.',
			},
		},
		{
			id: 'files',
			title: 'Files',
			recording: {
				src: '/docs/images/home/files-dark.mp4',
				poster: '/docs/images/home/files-dark-poster.webp',
				label: 'Recent items lists the Reports folder; Tab browses into it and the arrows move through its files.',
			},
		},
		{
			id: 'in-files',
			title: 'In files',
			recording: {
				src: '/docs/images/home/in-files-dark.mp4',
				poster: '/docs/images/home/in-files-dark-poster.webp',
				label: 'Typing in and Space switches to searching inside files with Spotlight.',
			},
		},
		{
			id: 'dictionary',
			title: 'Dictionary',
			recording: {
				src: '/docs/images/home/dictionary-dark.mp4',
				poster: '/docs/images/home/dictionary-dark-poster.webp',
				label: 'd and Space looks up serendipity in the dictionary.',
			},
		},
		{
			id: 'keywords',
			title: 'Keywords',
			key: '?',
			recording: {
				src: '/docs/images/home/keywords-dark.mp4',
				poster: '/docs/images/home/keywords-dark-poster.webp',
				label: '? lists every command with its keyword.',
			},
		},
		{
			id: 'clipboard',
			title: 'Clipboard',
			recording: {
				src: '/docs/images/home/clipboard-dark.mp4',
				poster: '/docs/images/home/clipboard-dark-poster.webp',
				label: 'clip and Space lists the clipboard history, text and images, newest first.',
			},
		},
	];

	// The tabs scripts/docs-glass.nu has recorded over real Liquid Glass; the others are headless
	// recordings over Flow.
	const captured: Record<string, unknown> = glass.clips;

	// Every tab's clip is in the page at once, stacked in one cell, so a switch never waits on a
	// request or a new element. The picked tab is `tab`; `shown` is the one on screen, which
	// follows it once its first frame can be drawn, so the clip before it stays up until then.
	let tab = $state('apps');
	let shown = $state('apps');
	let ready = $state<Record<string, boolean>>({});
	// Left until the page is idle, so the clips behind the first one load after it.
	let idle = $state(false);

	$effect(() => {
		const start = () => (idle = true);
		if ('requestIdleCallback' in window) {
			const handle = requestIdleCallback(start, { timeout: 3000 });
			return () => cancelIdleCallback(handle);
		}
		const handle = setTimeout(start, 1500);
		return () => clearTimeout(handle);
	});

	$effect(() => {
		if (ready[tab]) shown = tab;
	});

	const standIn = $derived(!(`home/${shown}` in captured));

	const selectTab = (id: string) => {
		tab = id;
	};

	// Arrow keys, Home and End move between the tabs and pick the one they land on, as WAI-ARIA's
	// tabs do.
	const roving = (ids: string[], current: string, select: (id: string) => void) => (event: KeyboardEvent) => {
		const at = ids.indexOf(current);
		const next = {
			ArrowRight: at + 1,
			ArrowDown: at + 1,
			ArrowLeft: at - 1,
			ArrowUp: at - 1,
			Home: 0,
			End: ids.length - 1,
		}[event.key];
		if (next === undefined) return;
		event.preventDefault();
		const id = ids[(next + ids.length) % ids.length]!;
		select(id);
		const group = event.currentTarget as HTMLElement;
		group.querySelector<HTMLElement>(`[data-id="${id}"]`)?.focus();
	};
</script>

<div class="demo">
	<div class="picker">
		<div
			class="choices"
			role="tablist"
			aria-label="Features"
			tabindex="-1"
			onkeydown={roving(
				tabs.map((choice) => choice.id),
				tab,
				selectTab,
			)}
		>
			{#each tabs as choice (choice.id)}
				<button
					type="button"
					role="tab"
					id={`demo-tab-${choice.id}`}
					data-id={choice.id}
					aria-selected={tab === choice.id}
					aria-controls="demo-stage"
					tabindex={tab === choice.id ? 0 : -1}
					onclick={() => selectTab(choice.id)}
				>
					{choice.title}{#if choice.key}&nbsp;<kbd>{choice.key}</kbd>{/if}
				</button>
			{/each}
		</div>
		<a class="more" href="/docs/#features">More features →</a>
	</div>
	<div class="stage" id="demo-stage" role="tabpanel" aria-labelledby={`demo-tab-${tab}`}>
		{#each tabs as choice (choice.id)}
			<div class="slot" class:shown={shown === choice.id} inert={shown !== choice.id}>
				<Clip
					class="stage-clip"
					src={choice.recording.src}
					poster={choice.recording.poster}
					width={680}
					height={578}
					preload={shown === choice.id || tab === choice.id || idle ? 'auto' : 'metadata'}
					active={shown === choice.id}
					onready={() => (ready[choice.id] = true)}
					label={choice.recording.label}
				/>
			</div>
		{/each}
	</div>
	<p class="note" aria-live="polite">
		{#if standIn}
			This tab is a headless recording, not Liquid Glass.
		{/if}
		<a href="/docs/features/appearance/#backdrop">Add a shader behind the query field →</a>
	</p>
</div>

<style>
	.demo {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 16px;
		max-width: 680px;
		margin: 56px auto 0;
	}

	.picker {
		display: flex;
		flex-wrap: wrap;
		justify-content: center;
		align-items: center;
		gap: 6px 12px;
	}

	.choices {
		display: flex;
		flex-wrap: wrap;
		justify-content: center;
		gap: 6px;
	}

	.choices button {
		padding: 5px 12px;
		border: 1px solid var(--border-strong);
		border-radius: 999px;
		background: transparent;
		color: var(--fg-muted);
		font: inherit;
		font-size: 13px;
		cursor: pointer;
		transition:
			background 150ms var(--ease-out),
			color 150ms var(--ease-out);
	}

	.choices button:hover {
		color: var(--fg);
		background: var(--bg-elev);
	}

	.choices button[aria-selected='true'],
	.choices button[aria-checked='true'] {
		color: var(--fg);
		background: var(--bg-card);
		border-color: var(--accent);
	}

	.choices button:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: 2px;
	}

	kbd {
		font: inherit;
		font-family: var(--font-mono);
		font-size: 0.92em;
	}

	.more {
		font-size: 13px;
		color: var(--fg-muted);
	}

	.more:hover {
		color: var(--fg);
	}

	/* As tall as the tallest recording, so switching never moves the page. */
	.stage {
		position: relative;
		width: 100%;
		aspect-ratio: 680 / 578;
	}

	.slot {
		position: absolute;
		inset: 0;
		z-index: 1;
		opacity: 0;
		/* The clip leaving stays up, under the one arriving, until that has faded in. */
		transition: opacity 0s linear 0.25s;
	}

	.slot.shown {
		z-index: 2;
		opacity: 1;
		transition: opacity 0.25s ease;
	}

	@media (prefers-reduced-motion: reduce) {
		.slot,
		.slot.shown {
			transition: none;
		}
	}

	.stage :global(.stage-clip) {
		position: absolute;
		inset: 0;
	}

	.stage :global(.stage-clip video) {
		height: 100%;
		object-fit: contain;
		object-position: top;
	}

	.note {
		margin: 0;
		min-height: 3em;
		max-width: 60ch;
		font-size: 13px;
		line-height: 1.5;
		color: var(--fg-dim);
		text-align: center;
		text-wrap: balance;
	}

	.note a {
		color: var(--fg-muted);
		text-decoration: underline;
		text-underline-offset: 3px;
	}
</style>
