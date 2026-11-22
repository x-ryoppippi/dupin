<script lang="ts">
	import { onMount } from 'svelte';
	import { backdrops, createRenderer, srgb, type Renderer } from './backdrop.ts';
	import { repo } from '../site.ts';
	import { marked, marking, scenarios, type Scenario } from './scenarios.ts';

	const [first] = scenarios;
	// The panel starts hidden; the demo opens it with the hotkey, as a person would.
	let query = $state('');
	let mode = $state<Scenario['mode']>();
	let shown = $state<Scenario>(first);
	let open = $state(false);
	let launching = $state(false);
	// File searches answer a moment after the key, as Spotlight does, so their rows follow `found`
	// while the field says "Searching…"; app results follow the query at once.
	let found = $state('');
	let searching = $state(false);
	let selected = $state(0);
	const rows = $derived(found ? (shown.steps[found.toLowerCase()] ?? []) : []);
	let keys = $state<{ id: number; label: string }[]>([]);
	let keysShown = $state(false);
	let desk: HTMLElement;
	let panel: HTMLElement;
	let field: HTMLElement;
	let canvas: HTMLCanvasElement;

	// What draws behind the panel: plain glass, as Dupin opens by default, or one of its shaders.
	let backdrop = $state('glass');
	let webgpu = $state<boolean | undefined>();
	let drawing = $state(false);
	let renderer: Renderer | undefined;
	let still = false;
	let seen = true;
	let frame = 0;
	let picks = 0;
	let shownAt = 0;
	let typedAt = -Infinity;
	const modes: Record<string, number> = { Files: 1, 'In files': 2 };
	const placeholders: Record<string, string> = { '': 'Search apps', Files: 'Search files', 'In files': 'Search inside files' };
	const hints: Record<string, string> = {
		Files: 'Type to search files with Spotlight  ·  ⌫ back to apps',
		'In files': 'Type to search inside files with Spotlight  ·  ⌫ back to apps',
	};

	const setQuery = (next: string) => {
		query = next;
		typedAt = performance.now();
	};
	const inputs = () => {
		const now = performance.now();
		return {
			time: still ? 10 : (now - shownAt) / 1000,
			keystroke: still ? 1e4 : Math.min((now - typedAt) / 1000, 1e4),
			query: [...query].length,
			searching: searching ? 1 : 0,
			mode: (mode && modes[mode]) ?? 0,
			field: field.offsetTop + field.offsetHeight,
		};
	};
	const draw = () => {
		if (renderer && drawing && seen && !document.hidden) renderer.draw(inputs());
	};
	const tick = () => {
		draw();
		frame = still ? 0 : requestAnimationFrame(tick);
	};

	const pick = async (id: string) => {
		backdrop = id;
		const pickNumber = ++picks;
		drawing = false;
		cancelAnimationFrame(frame);
		renderer?.clear();
		const choice = backdrops.find((b) => b.id === id);
		if (!renderer || !choice) return;
		const ok = await renderer.use(choice.source);
		if (pickNumber !== picks) return;
		drawing = ok;
		if (ok) {
			shownAt = performance.now();
			tick();
		}
	};

	onMount(() => {
		still = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
		let alive = true;
		const observer = new IntersectionObserver(([entry]) => (seen = entry?.isIntersecting ?? true));
		observer.observe(desk);
		const resized = new ResizeObserver(() => still && draw());
		resized.observe(panel);
		void createRenderer(canvas, srgb(getComputedStyle(desk).getPropertyValue('--dupin-accent'))).then((r) => {
			if (!alive) return r?.destroy();
			webgpu = r !== undefined;
			renderer = r;
		});
		return () => {
			alive = false;
			picks++;
			cancelAnimationFrame(frame);
			observer.disconnect();
			resized.disconnect();
			renderer?.destroy();
		};
	});

	onMount(() => {
		if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
			// One still search in place of the loop.
			open = true;
			query = first.query;
			found = first.query;
			return;
		}

		let alive = true;
		let visible = true;
		let keyId = 0;
		let hideTimer: ReturnType<typeof setTimeout> | undefined;
		const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
		const observer = new IntersectionObserver(([entry]) => (visible = entry?.isIntersecting ?? true));
		observer.observe(desk);

		// The keycast shows what was pressed; `alone` clears it first, so a shortcut reads by itself.
		const press = (label: string, alone = false) => {
			keys = [...(alone ? [] : keys), { id: keyId++, label }].slice(-8);
			keysShown = true;
			clearTimeout(hideTimer);
			hideTimer = setTimeout(() => {
				keysShown = false;
				hideTimer = setTimeout(() => (keys = []), 300);
			}, 1100);
		};
		let answer: ReturnType<typeof setTimeout> | undefined;
		const type = async (char: string) => {
			press(char);
			setQuery(query + char);
			selected = 0;
			clearTimeout(answer);
			if (mode) {
				searching = true;
				answer = setTimeout(() => {
					found = query;
					searching = false;
				}, 380);
			} else {
				found = query;
			}
			await sleep(120 + Math.random() * 80);
		};
		const down = async (alone = false) => {
			press('↓', alone);
			selected = Math.min(selected + 1, rows.length - 1);
			await sleep(420);
		};

		// Each round is one use of Dupin: ⌘Space shows the bar, the query narrows the list key by
		// key, and ↩ opens the top result and hides the panel. File searches enter their mode
		// first: Space on an empty query for file names, `in` then Space for contents.
		const play = async () => {
			await sleep(900);
			for (let i = 0; alive; i = (i + 1) % scenarios.length) {
				while (alive && (!visible || document.hidden)) await sleep(400);
				const scenario = scenarios[i]!;
				shown = scenario;
				press('⌘ Space', true);
				open = true;
				shownAt = performance.now();
				await sleep(1800);
				keys = [];
				if (scenario.mode === 'In files') {
					await type('i');
					await type('n');
					await sleep(300);
				}
				if (scenario.mode) {
					press('␣');
					setQuery('');
					found = '';
					mode = scenario.mode;
					// Long enough to read the hint the mode shows before the search starts.
					await sleep(1600);
					keys = [];
				}
				for (const char of scenario.query) await type(char);
				await sleep(1200);
				if (rows.length > 2) {
					await down(true);
					await down();
					await sleep(500);
				}
				press('↩', true);
				launching = true;
				await sleep(220);
				open = false;
				launching = false;
				query = '';
				found = '';
				selected = 0;
				mode = undefined;
				await sleep(1300);
			}
		};
		void play();

		return () => {
			alive = false;
			observer.disconnect();
			clearTimeout(hideTimer);
			clearTimeout(answer);
		};
	});
</script>

<div class="desk" bind:this={desk} aria-hidden="true">
	<div class={['panel', !open && 'is-closed']} bind:this={panel}>
		<canvas class={['backdrop', drawing && 'is-drawing']} bind:this={canvas}></canvas>
		<div class="field" bind:this={field}>
			{#if mode}<span class="chip">{mode}</span>{:else}<span class="glyph">⌕</span>{/if}
			<span class="text">
				{#if query}{query}<span class="caret"></span>{:else}<span class="caret"></span><span class="placeholder"
						>{placeholders[mode ?? ''] ?? placeholders['']}</span
					>{/if}
			</span>
			{#if searching}<span class="searching">Searching…</span>{/if}
		</div>
		{#if rows.length === 0 && mode && !query}
			<div class="hint">{hints[mode]}</div>
		{:else if rows.length === 0 && found && !searching}
			<div class="hint">No matches</div>
		{:else if rows.length > 0}
			<ul class="rows">
				{#each rows as row, i (row.title)}
					<li class={['row', i === selected && 'is-selected', i === selected && launching && 'is-launching']}>
						<span class={`app-icon app-icon--${row.icon}`} style:--tint={row.tint}
							>{#if row.glyph}<span class={row.glyph}></span>{/if}</span
						>
						<span class="row-text">
							<span class="row-title">
								{#each marked(row.title, row.alias ? '' : query, marking(mode)) as run, k (k)}{#if run.hit}<mark
											>{run.text}</mark
										>{:else}{run.text}{/if}{/each}
							</span>
							<span class="row-sub">
								{#if row.alias}<span class="alias">{row.alias}</span>{/if}{row.sub}
							</span>
						</span>
					</li>
				{/each}
			</ul>
		{/if}
	</div>
	<div class={['keycast', keysShown && 'is-shown']}>
		{#each keys as key (key.id)}<span>{key.label}</span>{/each}
	</div>
</div>
<div class="backdrop-picker" role="group" aria-label="Backdrop">
	<button type="button" aria-pressed={backdrop === 'glass'} onclick={() => pick('glass')}>Glass</button>
	{#each backdrops as b (b.id)}
		<button
			type="button"
			aria-pressed={backdrop === b.id}
			disabled={webgpu === false}
			title={webgpu === false ? 'Shaders need WebGPU, which this browser lacks' : undefined}
			onclick={() => pick(b.id)}>{b.title}</button
		>
	{/each}
	<a class="more" href={`${repo}/tree/main/examples/shaders`}>More shaders ↗</a>
</div>

<style>
	/* The panel in Dupin's own measures and colors (crates/dupin/src/view.rs and Rune's dark
	   theme), so the demo looks like the app: Rune's blue accent, which the shaders get too. */
	.desk {
		--dupin-accent: oklch(0.66 0.17 256);
		--dupin-text: oklch(0.955 0.003 265);
		--dupin-text-secondary: oklch(0.72 0.008 265);
		--dupin-separator: oklch(1 0 0 / 0.055);
	}

	.panel {
		max-width: 680px;
		margin: 0 auto;
		overflow: hidden;
		text-align: left;
		border-radius: 14px;
		color: var(--dupin-text);
		background: oklch(0.232 0.005 265 / 0.45);
		backdrop-filter: blur(30px) saturate(1.6);
		-webkit-backdrop-filter: blur(30px) saturate(1.6);
		box-shadow:
			inset 0 0 0 1px rgb(255 255 255 / 0.08),
			0 30px 80px -20px rgb(0 0 0 / 0.7);
	}

	.panel.is-closed {
		visibility: hidden;
	}

	.field {
		display: flex;
		align-items: center;
		gap: 12px;
		height: 64px;
		padding: 0 18px;
		font-size: 22px;
		line-height: 30px;
	}

	.field:not(:last-child) {
		border-bottom: 1px solid var(--dupin-separator);
	}

	.glyph {
		flex: none;
		font-size: 24px;
		line-height: 30px;
		color: var(--dupin-text-secondary);
	}

	.placeholder {
		color: var(--dupin-text-secondary);
	}

	.text {
		white-space: pre;
	}

	.caret {
		display: inline-block;
		width: 2px;
		height: 26px;
		margin: 0 -1px;
		vertical-align: -6px;
		background: var(--dupin-text);
	}

	.chip {
		flex: none;
		display: inline-flex;
		align-items: center;
		height: 28px;
		padding: 0 10px;
		border-radius: 7px;
		font-size: 13px;
		font-weight: 600;
		line-height: 1;
		color: var(--dupin-accent);
		background: oklch(from var(--dupin-accent) l c h / 0.16);
	}

	.searching {
		margin-left: auto;
		font-size: 12px;
		font-weight: 500;
		line-height: 1;
		color: var(--dupin-text-secondary);
	}

	.hint {
		white-space: pre;
		display: flex;
		align-items: center;
		height: 44px;
		padding: 0 20px;
		font-size: 13px;
		color: var(--dupin-text-secondary);
	}

	.rows {
		list-style: none;
		margin: 0;
		padding: 6px 0;
	}

	.row {
		display: flex;
		align-items: center;
		gap: 12px;
		height: 52px;
		margin: 0 6px;
		padding: 0 10px;
		border-radius: 8px;
	}

	.row-text {
		display: flex;
		flex-direction: column;
		gap: 2px;
		min-width: 0;
	}

	.row-title {
		font-size: 15px;
		font-weight: 500;
		line-height: 20px;
	}

	.row-title mark {
		background: none;
		font-weight: 700;
		color: var(--dupin-accent);
	}

	.row-sub {
		font-size: 12px;
		font-weight: 500;
		color: var(--dupin-text-secondary);
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.alias::after {
		content: ' · ';
	}

	.row.is-selected {
		color: #fff;
		background: var(--dupin-accent);
	}

	.row.is-selected .row-sub {
		color: rgb(255 255 255 / 0.78);
	}

	.row.is-selected .row-title mark {
		color: #fff;
	}

	.row.is-launching {
		filter: brightness(0.85);
	}

	.app-icon--glyph {
		display: grid;
		place-items: center;
		font-size: 20px;
		color: #fff;
		background: var(--tint);
	}

	/* The backdrop draws under the panel's content and inside its glass. */
	.panel {
		position: relative;
		isolation: isolate;
	}

	.backdrop {
		position: absolute;
		inset: 0;
		z-index: -1;
		width: 100%;
		height: 100%;
		border-radius: inherit;
		opacity: 0;
		transition: opacity 200ms var(--ease-out);
		pointer-events: none;
	}

	.backdrop.is-drawing {
		opacity: 1;
	}

	.backdrop-picker {
		display: flex;
		flex-wrap: wrap;
		justify-content: center;
		gap: 6px;
		margin: 16px auto 0;
	}

	.backdrop-picker button {
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

	.backdrop-picker button:hover:not(:disabled) {
		color: var(--fg);
		background: var(--bg-elev);
	}

	.backdrop-picker button[aria-pressed='true'] {
		color: var(--fg);
		background: var(--bg-card);
		border-color: var(--accent);
	}

	.backdrop-picker .more {
		align-self: center;
		margin-left: 6px;
		font-size: 13px;
		color: var(--fg-muted);
	}

	.backdrop-picker .more:hover {
		color: var(--fg);
	}

	.backdrop-picker button:disabled {
		opacity: 0.4;
		cursor: not-allowed;
	}
</style>
