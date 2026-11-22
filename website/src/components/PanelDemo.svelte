<script lang="ts">
	import { onMount } from 'svelte';
	import { backdrops, createRenderer, srgb, type Renderer } from './backdrop.ts';
	import { repo } from '../site.ts';
	import { scenarios, type Row, type Scenario } from './scenarios.ts';

	const [first] = scenarios;
	let query = $state(first.query);
	let mode = $state(first.mode);
	let rows = $state<Row[]>(first.rows);
	let open = $state(true);
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
			searching: mode && query && rows.length === 0 ? 1 : 0,
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
		if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;

		let alive = true;
		let visible = true;
		let keyId = 0;
		let hideTimer: ReturnType<typeof setTimeout> | undefined;
		const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
		const observer = new IntersectionObserver(([entry]) => (visible = entry?.isIntersecting ?? true));
		observer.observe(desk);

		const press = (label: string) => {
			keys = [...keys, { id: keyId++, label }].slice(-8);
			keysShown = true;
			clearTimeout(hideTimer);
			hideTimer = setTimeout(() => {
				keysShown = false;
				hideTimer = setTimeout(() => (keys = []), 300);
			}, 1100);
		};
		// Results follow every key, as Dupin's do: a scenario's rows with as much of each match
		// highlighted as has been typed.
		const matching = (scenario: Scenario, typed: string): Row[] =>
			typed ? scenario.rows.map((row) => ({ ...row, hit: row.hit?.slice(0, typed.length) })) : [];
		const type = async (char: string) => {
			press(char === ' ' ? '␣' : char);
			setQuery(query + char);
			await sleep(90 + Math.random() * 70);
		};

		// Replays the keys a person would press: ⌫ empties the query and, on an empty one, leaves a
		// mode; Space on an empty query enters file search, and `in` then Space enters content search.
		// After the last search, Esc closes the panel and the hotkey starts the round again.
		const play = async () => {
			let shown: Scenario = first;
			for (let i = 1; alive; i = (i + 1) % scenarios.length) {
				await sleep(2600);
				while (alive && (!visible || document.hidden)) await sleep(400);
				const scenario = scenarios[i]!;
				if (i === 0) {
					press('esc');
					open = false;
					await sleep(250);
					query = '';
					mode = undefined;
					rows = [];
					await sleep(1600);
					press('⌘ Space');
					open = true;
					shownAt = performance.now();
					await sleep(400);
				} else {
					while (query.length > 0) {
						press('⌫');
						setQuery(query.slice(0, -1));
						rows = matching(shown, query);
						await sleep(70);
					}
					rows = [];
					if (mode && mode !== scenario.mode) {
						press('⌫');
						mode = undefined;
						await sleep(250);
					}
				}
				if (scenario.mode && !mode) {
					if (scenario.mode === 'In files') {
						await type('i');
						await type('n');
						setQuery('');
					}
					press('␣');
					mode = scenario.mode;
					// Long enough to read the hint the mode shows before the search starts.
					await sleep(1600);
				}
				shown = scenario;
				for (const char of scenario.query) {
					const typing = type(char);
					rows = matching(scenario, query);
					await typing;
				}
			}
		};
		void play();

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
		</div>
		{#if rows.length === 0 && mode && !query}
			<div class="hint">{hints[mode]}</div>
		{:else if rows.length > 0}
			<ul class="rows">
				{#each rows as row, i (row.title)}
					<li class={['row', i === 0 && 'is-selected']}>
						<span class={`app-icon app-icon--${row.icon}`}></span>
						<span class="row-text">
							<span class="row-title">
								{#if row.hit}<mark>{row.hit}</mark>{row.title.slice(row.hit.length)}{:else}{row.title}{/if}
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
