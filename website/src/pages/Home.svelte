<script lang="ts">
	import CopyButton from '../components/CopyButton.svelte';
	import Demo from '../components/Demo.svelte';
	import DownloadButton from '../components/DownloadButton.svelte';
	import { choices } from '../components/docs/DesignChoices.svelte';
	import { steps } from '../components/docs/LauncherLoop.svelte';
	import Footer from '../components/Footer.svelte';
	import Nav from '../components/Nav.svelte';
	import { channels, downloadChannel, repo, setupPrompt, stableReleased } from '../site.ts';

	type Still = { src: string; width: number; height: number };

	// In the order the bento reads at every width; the grid areas below place each tile by its id.
	const features: { id: string; title: string; text: string; href: string; icon?: string; still?: Still }[] = [
		{
			id: 'apps',
			title: 'Apps and files',
			text: 'Apps by any name macOS knows them by. Space on an empty query searches files, `in` and Space searches inside them, and Tab browses a folder.',
			href: '/docs/features/apps/',
			icon: 'icon-[ph--squares-four]',
		},
		{
			id: 'keywords',
			title: 'Keywords',
			text: 'A keyword and Space reaches a command: `cal`, `d`, `clip`, `gh`. `?` lists every one, and you can add your own aliases and global shortcuts.',
			href: '/docs/features/keywords/',
			still: { src: '/docs/images/keywords/list-dark.png', width: 1360, height: 986 },
		},
		{
			id: 'clipboard',
			title: 'Clipboard and snippets',
			text: 'The text, images and files you copy, searchable and encrypted, and snippets that paste with the date or clipboard filled in.',
			href: '/docs/features/clipboard/',
			still: { src: '/docs/images/clipboard/history-dark.png', width: 1360, height: 1034 },
		},
		{
			id: 'commands',
			title: 'Commands',
			text: 'Your own scripts, sleep and lock, quitting apps, and Restart or Update Dupin, from search or a hotkey.',
			href: '/docs/features/scripts/',
			icon: 'icon-[ph--terminal-window]',
		},
		{
			id: 'answers',
			title: 'Answers as you type',
			text: 'Calculations and unit or currency conversions in the main search, your calendar under `cal`, the dictionary under `d`, emoji under `emoji`.',
			href: '/docs/features/calculator/',
			still: { src: '/docs/images/calculator/units-dark.png', width: 1360, height: 502 },
		},
		{
			id: 'extensions',
			title: 'Extensions',
			text: 'TypeScript packages installed from Git, sandboxed to the capabilities you grant and tested against fixtures, so an agent can write one.',
			href: '/docs/features/extensions/',
			still: { src: '/docs/images/extensions/form-dark.png', width: 1360, height: 716 },
		},
		{
			id: 'account',
			title: 'No account',
			text: 'What Dupin keeps stays on your Mac, encrypted, and can sync between your Macs through a folder you already sync.',
			href: '/docs/features/storage/',
			icon: 'icon-[ph--lock-simple]',
		},
		{
			id: 'snapshots',
			title: 'Snapshots and MCP',
			text: 'Save a calculation, search or extension view to read again later, and let an AI agent read and drive Dupin over MCP.',
			href: '/docs/features/snapshots/',
			still: { src: '/docs/images/snapshots/list-dark.png', width: 1360, height: 362 },
		},
	];

	// The keys docs/philosophy.md lists as working everywhere.
	const keys = ['↩', '⌘↩', '⌘K', '⌫', '?'];
</script>

{#snippet code(text: string)}
	{#each text.split('`') as part, i (i)}{#if i % 2}<code>{part}</code>{:else}{part}{/if}{/each}
{/snippet}

<Nav />
<main>
	<section class="hero">
		<div class="hero-grid" aria-hidden="true"></div>
		<div class="container hero-inner">
			<a class="pronunciation rise" style="--d: 0ms" href="/docs/name/">
				Dupin <span lang="fr">/dy.pɛ̃/</span>
			</a>
			<h1 class="display hero-title rise" style="--d: 0ms">Press ⌘Space. It's there.</h1>
			<p class="hero-sub rise" style="--d: 100ms">
				The doorway to your Mac: type a keyword, reach anything.<br />
				A fast launcher for Apple silicon, with extensions you or your agent can write.
			</p>
			<div class="cta rise" style="--d: 200ms">
				<div class="cta-buttons">
					<DownloadButton href={channels[downloadChannel].dmg} />
					<CopyButton
						class="btn-secondary"
						text={setupPrompt}
						label="Copy a setup prompt for your coding agent"
						copiedMessage="Copied the setup prompt. Paste it into your coding agent."
					>
						{#snippet children()}
							<span class="btn-icon icon-[ph--copy]" aria-hidden="true"></span>
							Copy setup prompt
						{/snippet}
						{#snippet copiedChildren()}
							<span class="btn-icon icon-[ph--check]" aria-hidden="true"></span>
							Copied. Paste it into your agent
						{/snippet}
					</CopyButton>
				</div>
				<a class="link-arrow" href={repo}>
					<span class="gh icon-[simple-icons--github]" aria-hidden="true"></span>
					Read the source (it's GPL) <span class="arrow">↗</span>
				</a>
				<p class="cta-more">
					macOS 14 or later ·
					{#if !stableReleased}Nightly build until the first release ·{/if}
					<a href="/docs/features/hotkeys/#freeing-the-hotkey">Free ⌘Space from Spotlight first</a> ·
					<a href="/download/">All downloads</a>
				</p>
			</div>
		</div>
		<div class="container rise" style="--d: 300ms">
			<Demo />
			<ul class="traits">
				<li>
					<strong>Under 3 ms</strong>
					<span
						>from a keystroke to the frame that shows its results, at the 95th percentile, measured in CI on every
						change. The panel is drawn before you press the hotkey.</span
					>
				</li>
				<li>
					<strong>Written in Rust</strong>
					<span>Native, drawn on the GPU. No web view, no Electron.</span>
				</li>
				<li>
					<strong>Open source</strong>
					<span>GPL-3.0-or-later. Read it, build it with Nix, change it.</span>
				</li>
			</ul>
		</div>
	</section>
	<section class="philosophy">
		<div class="container">
			<p class="principle">
				A launcher should be the doorway to your desktop, not a replacement for it.
			</p>
			<ul class="bento choices">
				{#each choices as choice (choice.id)}
					<li class="tile" style:grid-area={choice.id}>
						<div class="tile-text">
							{#if choice.id === 'extensions'}
								<span class="tile-icon icon-[ph--puzzle-piece]" aria-hidden="true"></span>
							{/if}
							<strong>{choice.title}</strong>
							<span>{choice.text}</span>
						</div>
						{#if choice.id === 'doorway'}
							<ol class="loop" aria-label="The loop Dupin protects">
								{#each steps as step (step.label)}
									<li>
										<span class="loop-label">{step.label}</span>
										<span class="loop-note">{step.note}</span>
									</li>
								{/each}
							</ol>
						{:else if choice.id === 'transient'}
							<div class="handoff" aria-hidden="true">
								<span class="box panel">The panel</span>
								<span class="arrow"></span>
								<span class="windows">
									<span class="box">Snapshots in Settings</span>
									<span class="box">Script Output window</span>
								</span>
							</div>
						{:else if choice.id === 'keys'}
							<div class="keys" aria-hidden="true">
								{#each keys as key (key)}<kbd>{key}</kbd>{/each}
							</div>
						{/if}
					</li>
				{/each}
			</ul>
			<a class="link-arrow" href="/docs/philosophy/">Read the philosophy <span class="arrow">→</span></a>
		</div>
	</section>
	<section class="features" aria-labelledby="features-title">
		<div class="container">
			<h2 id="features-title" class="section-title">What it does</h2>
			<ul class="bento feature-grid">
				{#each features as feature (feature.id)}
					<li class="feature-{feature.id}" style:grid-area={feature.id}>
						<a class="tile" href={feature.href}>
							<div class="tile-text">
								{#if feature.icon}
									<span class={['tile-icon', feature.icon]} aria-hidden="true"></span>
								{/if}
								<strong>{feature.title}</strong>
								<span>{@render code(feature.text)}</span>
							</div>
							{#if feature.still}
								<div class="still">
									<img
										src={feature.still.src}
										width={feature.still.width}
										height={feature.still.height}
										alt=""
										loading="lazy"
										decoding="async"
									/>
								</div>
							{/if}
						</a>
					</li>
				{/each}
			</ul>
			<a class="link-arrow" href="/docs/#features">Every feature <span class="arrow">→</span></a>
		</div>
	</section>
</main>
<Footer />

<style>
	.hero {
		position: relative;
		padding: 72px 0 80px;
		text-align: center;
		isolation: isolate;
	}

	.hero-grid {
		position: absolute;
		inset: 0;
		z-index: -1;
		background-image:
			linear-gradient(to right, rgba(255, 255, 255, 0.025) 1px, transparent 1px),
			linear-gradient(to bottom, rgba(255, 255, 255, 0.025) 1px, transparent 1px);
		background-size: 48px 48px;
		mask-image: radial-gradient(ellipse 80% 60% at 50% 35%, black 40%, transparent 100%);
	}

	.hero-inner {
		position: relative;
		display: flex;
		flex-direction: column;
		align-items: center;
	}

	.pronunciation {
		margin-top: 8px;
		font-size: 13px;
		color: var(--fg-dim);
		letter-spacing: 0.02em;
	}

	.pronunciation:hover {
		color: var(--fg-muted);
	}

	.hero-title {
		font-size: clamp(40px, 6.4vw, 84px);
		max-width: 20ch;
		margin: 20px auto 22px;
		text-wrap: balance;
	}

	.hero-sub {
		font-size: clamp(16px, 1.4vw, 19px);
		color: var(--fg-muted);
		max-width: 640px;
		line-height: 1.55;
		letter-spacing: -0.005em;
		margin: 0;
		text-wrap: balance;
	}

	.rise {
		animation: rise 0.7s var(--ease-out) both;
		animation-delay: var(--d, 0ms);
	}

	@keyframes rise {
		from {
			opacity: 0;
			transform: translateY(16px);
		}
	}

	.cta {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 16px;
		margin-top: 36px;
	}

	.cta-buttons {
		display: flex;
		flex-direction: column;
		align-items: stretch;
		gap: 12px;
	}

	.cta-buttons > :global(*) {
		justify-content: center;
	}

	.link-arrow {
		display: inline-flex;
		align-items: center;
		gap: 8px;
		font-size: 14px;
		color: var(--fg-muted);
		transition: color 0.18s ease;
	}

	.link-arrow:hover {
		color: var(--fg);
	}

	.gh {
		width: 14px;
		height: 14px;
	}

	.cta-more {
		margin: 0;
		font-size: 13px;
		color: var(--fg-dim);
	}

	.cta-more a {
		color: var(--fg-muted);
		text-decoration: underline;
		text-decoration-color: rgba(163, 163, 163, 0.4);
		text-underline-offset: 3px;
	}

	.cta-more a:hover {
		color: var(--fg);
	}

	.traits {
		display: grid;
		grid-template-columns: repeat(3, 1fr);
		gap: 32px;
		max-width: 1080px;
		margin: 40px auto 0;
		padding: 0;
		list-style: none;
		text-align: left;
	}

	.traits li {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}

	.traits strong {
		font-size: 20px;
		font-weight: 600;
		letter-spacing: -0.02em;
		color: var(--fg);
	}

	.traits span {
		font-size: 14px;
		line-height: 1.5;
		color: var(--fg-muted);
	}

	.philosophy,
	.features {
		padding: 0 0 112px;
	}

	.philosophy .container,
	.features .container {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 36px;
		max-width: 1120px;
	}

	.principle,
	.section-title {
		margin: 0;
		font-size: clamp(22px, 3vw, 32px);
		font-weight: 600;
		letter-spacing: -0.02em;
		text-align: center;
	}

	.principle {
		max-width: 28ch;
		line-height: 1.25;
		text-wrap: balance;
	}

	/* Each tile takes the grid area named after its id. */
	.bento {
		display: grid;
		width: 100%;
		margin: 0;
		padding: 0;
		list-style: none;
		grid-template-columns: repeat(4, minmax(0, 1fr));
		gap: 16px;
	}

	.choices {
		grid-template-areas:
			'doorway doorway transient transient'
			'doorway doorway keys extensions';
	}

	/*
	 * Twelve columns so a row's spans can follow its stills' aspect ratios: Answers (2.7:1) takes seven
	 * and Extensions (1.9:1) five, which brings both stills to about the same height. Equal halves
	 * would leave Answers some 80px short, stretched by the row into a gap.
	 */
	.feature-grid {
		grid-template-columns: repeat(12, minmax(0, 1fr));
		grid-template-areas:
			'apps apps apps apps apps apps keywords keywords keywords keywords keywords keywords'
			'clipboard clipboard clipboard clipboard clipboard clipboard keywords keywords keywords keywords keywords keywords'
			'clipboard clipboard clipboard clipboard clipboard clipboard commands commands commands commands commands commands'
			'answers answers answers answers answers answers answers extensions extensions extensions extensions extensions'
			'account account account snapshots snapshots snapshots snapshots snapshots snapshots snapshots snapshots snapshots';
	}

	.feature-grid li {
		display: flex;
	}

	.tile {
		display: flex;
		flex: 1;
		flex-direction: column;
		gap: 24px;
		min-width: 0;
		padding: 24px;
		overflow: hidden;
		border: 1px solid var(--border);
		border-radius: var(--radius-lg);
		background: radial-gradient(120% 80% at 0% 0%, rgba(255, 255, 255, 0.035), transparent 60%), var(--bg-elev);
		text-align: left;
		transition: border-color 0.18s ease;
	}

	a.tile:hover {
		border-color: var(--border-strong);
	}

	.tile-text {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.tile-icon {
		width: 22px;
		height: 22px;
		margin-bottom: 8px;
		color: var(--fg-dim);
	}

	.tile strong {
		font-size: 16px;
		font-weight: 600;
		letter-spacing: -0.01em;
		color: var(--fg);
	}

	.tile-text > span:last-child {
		max-width: 52ch;
		font-size: 14px;
		line-height: 1.55;
		color: var(--fg-muted);
	}

	.tile code {
		font-size: 0.92em;
	}

	.still {
		mask-image: linear-gradient(to bottom, black 70%, transparent);
	}

	.still img {
		display: block;
		width: 100%;
		height: auto;
		border-radius: 10px;
		transition: transform 0.4s var(--ease-out);
	}

	a.tile:hover .still img {
		transform: translateY(-4px);
	}

	/* A tile wide enough sets its short still beside the text rather than under it. */
	.feature-snapshots .tile {
		display: grid;
		grid-template-columns: minmax(0, 1fr) minmax(0, 2fr);
		align-items: center;
	}

	.feature-snapshots .still {
		mask-image: none;
	}

	/* The doorway tile spans two rows that the tiles beside it size, so its steps spread to fill them. */
	.loop {
		display: flex;
		flex-grow: 1;
		flex-direction: column;
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.loop li {
		position: relative;
		display: flex;
		flex-direction: column;
		gap: 2px;
		padding: 0 0 20px 28px;
	}

	.loop li:not(:last-child) {
		flex-grow: 1;
	}

	.loop li:last-child {
		padding-bottom: 0;
	}

	.loop li::before {
		content: '';
		position: absolute;
		top: 5px;
		left: 0;
		width: 9px;
		height: 9px;
		border: 1px solid var(--accent);
		border-radius: 999px;
		background: color-mix(in srgb, var(--accent) 35%, transparent);
	}

	.loop li:not(:last-child)::after {
		content: '';
		position: absolute;
		top: 20px;
		bottom: 4px;
		left: 5px;
		width: 1px;
		background: linear-gradient(var(--fg-faint), var(--border));
	}

	.loop li:last-child::before {
		border-style: dashed;
		border-color: var(--fg-faint);
		background: transparent;
	}

	.loop-label {
		font-size: 14px;
		font-weight: 600;
		color: var(--fg);
	}

	.loop li:last-child .loop-label {
		color: var(--fg-muted);
	}

	.loop-note {
		font-size: 13px;
		line-height: 1.5;
		color: var(--fg-dim);
	}

	.handoff {
		display: grid;
		grid-template-columns: minmax(0, 1fr) 40px minmax(0, 1.3fr);
		align-items: center;
		margin-top: auto;
	}

	.handoff .box {
		padding: 10px 12px;
		border: 1px solid var(--border-strong);
		border-radius: var(--radius-sm);
		background: var(--bg-card);
		font-size: 13px;
		color: var(--fg-muted);
	}

	.handoff .panel {
		border-style: dashed;
		text-align: center;
	}

	.handoff .windows {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.handoff .arrow {
		position: relative;
		height: 1px;
		margin: 0 6px;
		background: var(--fg-faint);
	}

	.handoff .arrow::after {
		content: '';
		position: absolute;
		right: 0;
		top: -3px;
		width: 6px;
		height: 6px;
		border-top: 1px solid var(--fg-faint);
		border-right: 1px solid var(--fg-faint);
		transform: rotate(45deg);
	}

	.keys {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
		margin-top: auto;
	}

	.keys kbd {
		min-width: 36px;
		padding: 6px 9px;
		border: 1px solid var(--border-strong);
		border-bottom-width: 3px;
		border-radius: var(--radius-sm);
		background: var(--bg-card);
		font-family: var(--font-mono);
		font-size: 13px;
		color: var(--fg);
		text-align: center;
	}

	@media (max-width: 1000px) {
		.bento {
			grid-template-columns: repeat(2, minmax(0, 1fr));
		}

		.choices {
			grid-template-areas:
				'doorway doorway'
				'transient transient'
				'keys extensions';
		}

		/* Two columns are too narrow to balance Answers against Extensions, so each takes a row. */
		.feature-grid {
			grid-template-columns: repeat(2, minmax(0, 1fr));
			grid-template-areas:
				'apps keywords'
				'clipboard keywords'
				'clipboard commands'
				'answers answers'
				'extensions extensions'
				'account snapshots';
		}

		.feature-answers .tile,
		.feature-extensions .tile {
			display: grid;
			grid-template-columns: minmax(0, 1fr) minmax(0, 2fr);
			align-items: center;
		}

		.feature-snapshots .tile {
			display: flex;
		}

		.feature-snapshots .still {
			mask-image: linear-gradient(to bottom, black 70%, transparent);
		}
	}

	@media (max-width: 820px) {
		.traits {
			grid-template-columns: 1fr;
			gap: 20px;
		}
	}

	@media (max-width: 640px) {
		.bento {
			grid-template-columns: minmax(0, 1fr);
			gap: 12px;
		}

		.choices {
			grid-template-areas: 'doorway' 'transient' 'keys' 'extensions';
		}

		.feature-grid {
			grid-template-areas: 'apps' 'keywords' 'clipboard' 'commands' 'answers' 'extensions' 'account' 'snapshots';
		}

		.tile,
		.feature-answers .tile,
		.feature-extensions .tile {
			display: flex;
			padding: 20px;
		}

		.handoff {
			grid-template-columns: minmax(0, 1fr) 28px minmax(0, 1.4fr);
		}
	}
</style>
