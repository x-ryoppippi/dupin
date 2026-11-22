<script lang="ts">
	import { onMount } from 'svelte';
	import { backdrops, createRenderer, srgb, type Renderer } from './backdrop.ts';
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
		void createRenderer(canvas, srgb(getComputedStyle(desk).getPropertyValue('--accent'))).then((r) => {
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
					await sleep(350);
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
			<span class="field-icon icon-[ph--magnifying-glass]"></span>
			{#if mode}<span class="chip">{mode}</span>{/if}
			<span class="query">{query}</span>
			<span class="caret"></span>
		</div>
		{#if rows.length > 0}
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
						{#if i === 0}<kbd class="kbd">↩</kbd>{/if}
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
</div>

<style>
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

	.backdrop-picker button:disabled {
		opacity: 0.4;
		cursor: not-allowed;
	}
</style>
