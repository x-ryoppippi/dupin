<script lang="ts">
	import glass from '../../../docs/images/glass.json';
	import { repo } from '../site.ts';
	import Clip from './Clip.svelte';

	// The home page's recordings from scripts/docs-images/images.toml: a tab per feature over
	// Liquid Glass, and the Apps tab again over each backdrop shader. Only the one shown is in the
	// page, so the rest load when picked.
	type Recording = { src: string; poster: string; width: number; height: number; label: string };

	const tabs: { id: string; title: string; key?: string; recording: Recording }[] = [
		{
			id: 'apps',
			title: 'Apps',
			recording: {
				src: '/docs/images/home/apps-dark.mp4',
				poster: '/docs/images/home/apps-dark-poster.webp',
				width: 680,
				height: 554,
				label: 'Typing maps lists Maps first; ⌘K opens its actions.',
			},
		},
		{
			id: 'files',
			title: 'Files',
			recording: {
				src: '/docs/images/home/files-dark.mp4',
				poster: '/docs/images/home/files-dark-poster.webp',
				width: 680,
				height: 450,
				label: 'Recent items lists the Reports folder; Tab browses into it and the arrows move through its files.',
			},
		},
		{
			id: 'in-files',
			title: 'In files',
			recording: {
				src: '/docs/images/home/in-files-dark.mp4',
				poster: '/docs/images/home/in-files-dark-poster.webp',
				width: 680,
				height: 554,
				label: 'Typing in and Space switches to searching inside files with Spotlight.',
			},
		},
		{
			id: 'dictionary',
			title: 'Dictionary',
			recording: {
				src: '/docs/images/home/dictionary-dark.mp4',
				poster: '/docs/images/home/dictionary-dark-poster.webp',
				width: 680,
				height: 554,
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
				width: 680,
				height: 554,
				label: '? lists every command with its keyword.',
			},
		},
		{
			id: 'clipboard',
			title: 'Clipboard',
			recording: {
				src: '/docs/images/home/clipboard-dark.mp4',
				poster: '/docs/images/home/clipboard-dark-poster.webp',
				width: 680,
				height: 578,
				label: 'clip and Space lists the clipboard history, text and images, newest first.',
			},
		},
	];

	const apps = 'Typing maps lists Maps first; ⌘K opens its actions';
	const backdrops: { id: string; title: string; text: string; recording?: Recording }[] = [
		{ id: 'glass', title: 'Glass', text: 'Liquid Glass, as Dupin opens: no shader.' },
		{
			id: 'flow',
			title: 'Flow',
			text: 'Built in: the accent colour as warped noise, swelling as you type.',
			recording: {
				src: '/docs/images/home/apps-flow-dark.mp4',
				poster: '/docs/images/home/apps-flow-dark-poster.webp',
				width: 680,
				height: 554,
				label: `${apps}, over the Flow backdrop.`,
			},
		},
		{
			id: 'aurora',
			title: 'Aurora',
			text: 'Curtains of light with twinkling stars.',
			recording: {
				src: '/docs/images/home/apps-aurora-dark.mp4',
				poster: '/docs/images/home/apps-aurora-dark-poster.webp',
				width: 680,
				height: 554,
				label: `${apps}, over the Aurora backdrop.`,
			},
		},
		{
			id: 'embers',
			title: 'Embers',
			text: 'Sparks rising from a flickering fire.',
			recording: {
				src: '/docs/images/home/apps-embers-dark.mp4',
				poster: '/docs/images/home/apps-embers-dark-poster.webp',
				width: 680,
				height: 554,
				label: `${apps}, over the Embers backdrop.`,
			},
		},
		{
			id: 'snow',
			title: 'Snow',
			text: 'Flakes drifting down in three layers.',
			recording: {
				src: '/docs/images/home/apps-snow-dark.mp4',
				poster: '/docs/images/home/apps-snow-dark-poster.webp',
				width: 680,
				height: 554,
				label: `${apps}, over the Snow backdrop.`,
			},
		},
		{
			id: 'matrix',
			title: 'Matrix',
			text: 'Columns of glyphs raining down, brightening as you type.',
			recording: {
				src: '/docs/images/home/apps-matrix-dark.mp4',
				poster: '/docs/images/home/apps-matrix-dark-poster.webp',
				width: 680,
				height: 554,
				label: `${apps}, over the Matrix backdrop.`,
			},
		},
	];

	// The tabs scripts/docs-glass.nu has recorded over real Liquid Glass; the others are headless
	// recordings over Flow.
	const captured: Record<string, unknown> = glass.clips;

	let tab = $state('apps');
	let backdrop = $state('glass');

	const shownTab = $derived(tabs.find((choice) => choice.id === tab) ?? tabs[0]);
	const shownBackdrop = $derived(backdrops.find((choice) => choice.id === backdrop) ?? backdrops[0]);
	const recording = $derived(shownBackdrop.recording ?? shownTab.recording);
	const standIn = $derived(backdrop === 'glass' && !(`home/${tab}` in captured));

	const selectTab = (id: string) => {
		tab = id;
		// A shader is recorded only on the Apps tab, so another tab shows its glass recording.
		if (id !== 'apps') backdrop = 'glass';
	};
	const selectBackdrop = (id: string) => {
		backdrop = id;
		if (id !== 'glass') tab = 'apps';
	};

	// Arrow keys, Home and End move between a group's choices and pick the one they land on, as
	// WAI-ARIA's tabs and radio groups do.
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
		{#key recording.src}
			<Clip
				class="stage-clip"
				src={recording.src}
				poster={recording.poster}
				width={recording.width}
				height={recording.height}
				preload="auto"
				label={recording.label}
			/>
		{/key}
	</div>
	<div class="picker">
		<div
			class="choices"
			role="radiogroup"
			aria-label="Backdrop"
			tabindex="-1"
			onkeydown={roving(
				backdrops.map((choice) => choice.id),
				backdrop,
				selectBackdrop,
			)}
		>
			{#each backdrops as choice (choice.id)}
				<button
					type="button"
					role="radio"
					data-id={choice.id}
					aria-checked={backdrop === choice.id}
					tabindex={backdrop === choice.id ? 0 : -1}
					onclick={() => selectBackdrop(choice.id)}>{choice.title}</button
				>
			{/each}
		</div>
		<a class="more" href={`${repo}/tree/main/examples/shaders`}>More shaders ↗</a>
	</div>
	<p class="note" aria-live="polite">
		{shownBackdrop.text}
		{#if standIn}
			This tab is a headless recording over the Flow shader, not Liquid Glass.
		{:else if backdrop !== 'glass'}
			Shaders are shown on the Apps tab. Pick one in Settings → Appearance, or
			<a href="/docs/features/shaders/">write your own in WGSL</a>.
		{/if}
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
