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
	let actionsOpen = $state(false);
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
		const pickNumber = ++picks;
		const choice = backdrops.find((b) => b.id === id);
		const pipeline = renderer && choice ? await renderer.compile(choice.source) : undefined;
		if (pickNumber !== picks) return;
		drawing = false;
		cancelAnimationFrame(frame);
		renderer?.use(pipeline);
		renderer?.clear();
		// The pressed button names what draws: a shader that didn't compile leaves plain glass.
		backdrop = pipeline ? id : 'glass';
		if (pipeline) {
			drawing = true;
			shownAt = performance.now();
			tick();
		}
	};

	onMount(() => {
		still = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
		let alive = true;
		// A still frame drawn while hidden is skipped, so draw it again once it can be seen.
		const redraw = () => still && draw();
		const observer = new IntersectionObserver(([entry]) => {
			seen = entry?.isIntersecting ?? true;
			redraw();
		});
		observer.observe(desk);
		const resized = new ResizeObserver(redraw);
		resized.observe(panel);
		document.addEventListener('visibilitychange', redraw);
		// A lost GPU device stops drawing for good: back to Glass, with the shaders disabled.
		const lost = () => {
			picks++;
			cancelAnimationFrame(frame);
			drawing = false;
			backdrop = 'glass';
			webgpu = false;
			renderer = undefined;
		};
		void createRenderer(canvas, srgb(getComputedStyle(desk).getPropertyValue('--dupin-accent')), lost)
			.catch(() => undefined)
			.then((r) => {
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
			document.removeEventListener('visibilitychange', redraw);
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
		// A wait that outlives the component ends the replay instead of resuming it.
		const stopped = new Error('stopped');
		const sleep = (ms: number) =>
			new Promise<void>((resolve, reject) => setTimeout(() => (alive ? resolve() : reject(stopped)), ms));
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
		const type = async (char: string) => {
			press(char);
			setQuery(query + char);
			selected = 0;
			if (mode) {
				// Spotlight answers each key a moment later; the next key waits for it, so every
				// prefix's results show.
				searching = true;
				await sleep(260);
				found = query;
				searching = false;
				await sleep(90);
			} else {
				found = query;
				await sleep(120 + Math.random() * 80);
			}
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
				if (i === 0) {
					press('⌘A', true);
					await sleep(250);
					press('⌫');
					setQuery('');
					found = '';
					await sleep(800);
					press('↑', true);
					setQuery(scenario.query);
					found = scenario.query;
					await sleep(1200);
				}
				if (i !== 0 && rows.length > 2) {
					await down(true);
					await down();
					await sleep(500);
				}
				if (scenario.actions) {
					press('→', true);
					actionsOpen = true;
					await sleep(1800);
					press('Esc', true);
					actionsOpen = false;
					await sleep(600);
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
				actionsOpen = false;
				await sleep(1300);
			}
		};
		play().catch((e: unknown) => {
			if (e !== stopped) throw e;
		});

		return () => {
			alive = false;
			observer.disconnect();
			clearTimeout(hideTimer);
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
								{#each marked(row.title, row.alias ? '' : found, marking(mode)) as run, k (k)}{#if run.hit}<mark
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
	{#if actionsOpen}
		<div class="action-menu">
			<div>Open</div>
			<div>Reveal in Finder</div>
			<div>Copy path</div>
		</div>
	{/if}
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
			disabled={!webgpu}
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
		position: relative;
		--dupin-accent: oklch(0.66 0.17 256);
		--dupin-text: oklch(0.955 0.003 265);
		--dupin-text-secondary: oklch(0.72 0.008 265);
		--dupin-separator: oklch(1 0 0 / 0.055);
	}

	.action-menu {
		position: absolute;
		top: calc(var(--desk-pad) + 64px);
		left: max(20px, calc((100% - 680px) / 2 + 20px));
		z-index: 2;
		min-width: 200px;
		padding: 5px;
		border: 1px solid rgb(255 255 255 / 0.16);
		border-radius: 7px;
		background: #303034;
		box-shadow: 0 8px 20px rgb(0 0 0 / 0.35);
		color: white;
		font-size: 14px;
		line-height: 24px;
	}

	.action-menu > div {
		padding: 0 12px;
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
		box-shadow:
			inset 0 0 0 1px rgb(255 255 255 / 0.08),
			0 0 0 1px rgb(0 0 0 / 0.6),
			0 8px 24px -8px rgb(0 0 0 / 0.5);
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
		flex: 1 1 auto;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: pre;
	}

	.caret {
		display: inline-block;
		width: 2px;
		height: 24px;
		margin: 0 -1px;
		vertical-align: -5px;
		background: #fff;
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
		white-space: pre-wrap;
		display: flex;
		align-items: center;
		min-height: 44px;
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

	.desk {
		position: relative;
		max-width: 1080px;
		margin: 64px auto 0;
		/* Fixed so the page never shifts while the panel grows and shrinks with its results. */
		height: calc(494px + var(--desk-pad) + 84px);
		padding: var(--desk-pad) 20px 0;
		--desk-pad: clamp(32px, 6vw, 72px);
		border-radius: var(--radius);
		box-shadow: 0 0 0 1px var(--border-strong);
		overflow: hidden;
		background:
			radial-gradient(
				ellipse 60% 70% at 30% 20%,
				oklch(0.36 0.12 var(--accent-h) / 0.9),
				transparent 70%
			),
			radial-gradient(ellipse 50% 60% at 80% 90%, oklch(0.32 0.1 200 / 0.8), transparent 70%), #07070a;
	}

	.app-icon {
		flex: none;
		width: 32px;
		height: 32px;
		border-radius: 8px;
		box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.12);
	}

	.app-icon--claude {
		background: #faf9f5 url('/apps/claude.svg') center / 66% no-repeat;
	}

	.app-icon--clock {
		background:
			linear-gradient(#111, #111) 50% 30% / 2px 40% no-repeat,
			linear-gradient(#111, #111) 66% 50% / 32% 2px no-repeat,
			radial-gradient(circle, #fff 0 62%, #1c1c1e 64%);
	}

	.app-icon--t3code {
		background: url('/apps/t3code.png') center / 125% no-repeat;
	}

	.app-icon--pdf,
	.app-icon--doc,
	.app-icon--sheet {
		position: relative;
		border-radius: 4px;
		width: 26px;
		margin: 0 3px;
		background: #f5f5f5;
	}

	.app-icon--pdf::after,
	.app-icon--doc::after,
	.app-icon--sheet::after {
		position: absolute;
		inset: auto 0 4px;
		text-align: center;
		font-family: var(--font-mono);
		font-size: 7px;
		font-weight: 700;
		color: #fff;
	}

	.app-icon--pdf::after {
		content: 'PDF';
		margin: 0 3px;
		border-radius: 2px;
		background: #dc2626;
	}

	.app-icon--doc::after {
		content: 'MD';
		color: #525252;
	}

	.app-icon--sheet::after {
		content: 'NUM';
		margin: 0 3px;
		border-radius: 2px;
		background: #16a34a;
	}

	.app-icon--chatgpt {
		background: #000 url('/apps/openai.svg') center / 70% no-repeat;
	}

	.keycast {
		position: absolute;
		left: 50%;
		bottom: clamp(16px, 3vw, 28px);
		display: flex;
		gap: 6px;
		padding: 8px 12px;
		border-radius: 12px;
		background: rgba(0, 0, 0, 0.55);
		backdrop-filter: blur(12px);
		box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.1);
		font-size: 20px;
		font-weight: 500;
		line-height: 1;
		color: var(--fg);
		opacity: 0;
		translate: -50% 6px;
		transition:
			opacity 0.2s ease,
			translate 0.2s ease;
		pointer-events: none;
	}

	.keycast.is-shown {
		opacity: 1;
		translate: -50% 0;
	}

	.keycast span {
		min-width: 0.7em;
		text-align: center;
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
