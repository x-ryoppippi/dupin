<script lang="ts">
	import { onMount } from 'svelte';
	import { scenarios, type Row } from './scenarios.ts';

	const [first] = scenarios;
	let query = $state(first.query);
	let mode = $state(first.mode);
	let rows = $state<Row[]>(first.rows);
	let open = $state(true);
	let keys = $state<{ id: number; label: string }[]>([]);
	let keysShown = $state(false);
	let desk: HTMLElement;

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
		const type = async (char: string) => {
			press(char === ' ' ? '␣' : char);
			query += char;
			await sleep(90 + Math.random() * 70);
		};

		// Replays the keys a person would press: Esc closes the panel and the hotkey opens it on an empty
		// query; Space on an empty query enters file search, and `in` then Space enters content search.
		const play = async () => {
			for (let i = 1; alive; i = (i + 1) % scenarios.length) {
				await sleep(2600);
				while (alive && (!visible || document.hidden)) await sleep(400);
				const scenario = scenarios[i]!;
				press('esc');
				open = false;
				await sleep(250);
				query = '';
				mode = undefined;
				rows = [];
				await sleep(900);
				press('⌘ Space');
				open = true;
				await sleep(400);
				if (scenario.mode) {
					if (scenario.mode === 'In files') {
						await type('i');
						await type('n');
						query = '';
					}
					press('␣');
					mode = scenario.mode;
					await sleep(350);
				}
				for (const char of scenario.query) await type(char);
				await sleep(120);
				rows = scenario.rows;
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
	<div class={['panel', !open && 'is-closed']}>
		<div class="field">
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
