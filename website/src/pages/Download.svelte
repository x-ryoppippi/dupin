<script lang="ts">
	import version from '../../../VERSION?raw';
	import CopyButton from '../components/CopyButton.svelte';
	import Footer from '../components/Footer.svelte';
	import Nav from '../components/Nav.svelte';
	import { channels, downloadChannel, freeHotkeySteps, nixInstall, releasesUrl, repo, stableReleased } from '../site.ts';

	const stableVersion = version.trim();
	let channel = $state<keyof typeof channels>(downloadChannel);
	const unreleased = $derived(channel === 'stable' && !stableReleased);
	const current = $derived(channels[channel]);
	const brew = $derived(`brew install --cask ${current.cask}`);
</script>

<Nav />
<main class="download">
	<header class="download-head">
		<h1 class="display download-title">
			<img class="download-icon" src="/favicon.svg" alt="" width="56" height="56" />
			Download Dupin
		</h1>
		<div class="segmented" role="radiogroup" aria-label="Release channel">
			{#each Object.entries(channels) as [key, value] (key)}
				<button
					type="button"
					role="radio"
					aria-checked={channel === key}
					class:is-active={channel === key}
					onclick={() => (channel = key as keyof typeof channels)}>{value.label}</button
				>
			{/each}
		</div>
		<p class="download-version">
			{#if unreleased}
				Not released yet
			{:else if channel === 'stable'}
				<a href={`${repo}/releases/tag/v${stableVersion}`}>v{stableVersion} <span class="arrow">↗</span></a>
			{:else}
				<a href={channels.nightly.release}>Built daily from main <span class="arrow">↗</span></a>
			{/if}
		</p>
	</header>

	{#if unreleased}
		<section class="download-section download-unreleased">
			<p>The first stable release isn't out yet. Until it is, the nightly build is the one to install.</p>
			<button type="button" class="btn-secondary" onclick={() => (channel = 'nightly')}>Show the nightly build</button>
		</section>
	{:else}
		<section class="download-section">
			<h2><span class="section-icon icon-[simple-icons--apple]" aria-hidden="true"></span>macOS</h2>
			<div class="download-cards">
				<a class="download-card" href={current.dmg}>
					<img src="/favicon.svg" alt="" width="32" height="32" />
					<span>
						<strong>Apple silicon</strong>
						<small>.dmg · macOS 14 or later</small>
					</span>
				</a>
			</div>
			<p class="download-note">
				Signed and notarized. Drag it to Applications and it updates itself. Official builds send <a href="/docs/features/telemetry/">one anonymous usage ping a day</a>, which one switch turns off.
			</p>
		</section>

		<section class="download-section">
			<h2><span class="section-icon icon-[simple-icons--homebrew]" aria-hidden="true"></span>Homebrew</h2>
			<div class="command">
				<code>{brew}</code>
				<CopyButton text={brew} label="Copy the Homebrew command" copiedMessage="Copied the Homebrew command." />
			</div>
		</section>
	{/if}

	<section class="download-section">
		<h2><span class="section-icon icon-[simple-icons--nixos]" aria-hidden="true"></span>Nix</h2>
		<div class="command">
			<code>{nixInstall}</code>
			<CopyButton text={nixInstall} label="Copy the Nix command" copiedMessage="Copied the Nix command." />
		</div>
		<p class="download-note">Builds main. A Nix install never updates itself; upgrade it through Nix.</p>
	</section>

	<section class="download-section">
		<h2><span class="section-icon icon-[ph--command]" aria-hidden="true"></span>Free ⌘Space first</h2>
		<p class="download-note">macOS gives a shortcut to only one app, so take ⌘Space back before you open Dupin.</p>
		<ol class="download-steps">
			{#each freeHotkeySteps as step (step)}<li>{step}</li>{/each}
		</ol>
	</section>

	<p class="download-all">All builds are on the <a href={releasesUrl}>GitHub releases page <span class="arrow">↗</span></a></p>
</main>
<Footer />

<style>
	.download {
		max-width: 760px;
		margin: 0 auto;
		padding: 72px 20px 96px;
	}

	.download-head {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 20px;
		margin-bottom: 64px;
	}

	.download-title {
		display: flex;
		align-items: center;
		gap: 16px;
		font-size: clamp(32px, 5vw, 48px);
	}

	.download-icon {
		width: 56px;
		height: 56px;
	}

	.segmented {
		display: inline-flex;
		padding: 4px;
		border: 1px solid var(--border-strong);
		border-radius: 999px;
		background: var(--bg-elev);
	}

	.segmented button {
		padding: 8px 22px;
		border: 0;
		border-radius: 999px;
		background: transparent;
		color: var(--fg-muted);
		font: inherit;
		font-size: 14px;
		font-weight: 500;
		cursor: pointer;
	}

	.segmented button.is-active {
		background: rgba(255, 255, 255, 0.1);
		color: var(--fg);
	}

	.download-version {
		margin: 0;
		font-size: 13px;
	}

	.download-version a,
	.download-all a,
	.download-note a {
		color: var(--fg-muted);
		text-decoration: underline;
		text-decoration-color: rgba(163, 163, 163, 0.4);
		text-underline-offset: 3px;
	}

	.download-version a:hover,
	.download-all a:hover,
	.download-note a:hover {
		color: var(--fg);
	}

	.download-section + .download-section {
		margin-top: 48px;
	}

	.download-section h2 {
		display: flex;
		align-items: center;
		gap: 10px;
		margin: 0 0 14px;
		padding-bottom: 12px;
		border-bottom: 1px solid var(--border);
		font-size: 18px;
		font-weight: 500;
		letter-spacing: -0.01em;
	}

	.section-icon {
		width: 18px;
		height: 18px;
		color: var(--fg-muted);
	}

	.download-cards {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
		gap: 10px;
	}

	.download-card {
		display: flex;
		align-items: center;
		gap: 14px;
		padding: 16px 18px;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		background: var(--bg-elev);
		transition:
			border-color 0.18s ease,
			background 0.18s ease;
	}

	.download-card:hover {
		border-color: var(--border-strong);
		background: rgba(255, 255, 255, 0.03);
	}

	.download-card span {
		display: flex;
		flex-direction: column;
		gap: 2px;
	}

	.download-card strong {
		font-size: 15px;
		font-weight: 600;
	}

	.download-card small {
		font-size: 13px;
		color: var(--fg-dim);
	}

	.download-note {
		margin: 12px 0 0;
		font-size: 13px;
		color: var(--fg-dim);
	}

	.command {
		display: flex;
		align-items: center;
		gap: 12px;
		max-width: 100%;
		width: fit-content;
		padding: 6px 6px 6px 16px;
		border: 1px solid color-mix(in srgb, var(--accent) 35%, transparent);
		border-radius: var(--radius-sm);
		background: color-mix(in srgb, var(--accent) 6%, var(--bg));
	}

	.command code {
		font-size: 13px;
		white-space: nowrap;
		overflow-x: auto;
		scrollbar-width: none;
	}

	.command :global(.copy-button) {
		flex: none;
		width: 32px;
		height: 32px;
	}

	.download-all {
		margin: 72px 0 0;
		text-align: center;
		font-size: 13px;
		color: var(--fg-dim);
	}

	.download-unreleased {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: 16px;
		color: var(--fg-muted);
	}

	.download-unreleased p {
		margin: 0;
	}

	.download-steps {
		margin: 12px 0 0;
		padding-left: 20px;
		display: grid;
		gap: 6px;
		color: var(--fg-muted);
		font-size: 15px;
	}
</style>
