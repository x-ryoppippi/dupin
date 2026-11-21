<script lang="ts">
	import version from '../../../VERSION?raw';
	import CopyButton from '../components/CopyButton.svelte';
	import Footer from '../components/Footer.svelte';
	import Nav from '../components/Nav.svelte';
	import { channels, downloadChannel, nixInstall, releasesUrl, repo, stableReleased } from '../site.ts';

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
			<h2><svg class="section-icon" viewBox="0 0 16 16" aria-hidden="true"><path fill="currentColor" d="M11.2 8.5c0-1.6 1.3-2.4 1.4-2.4-.8-1.1-1.9-1.3-2.3-1.3-1-.1-1.9.6-2.4.6-.5 0-1.3-.6-2.1-.6-1.1 0-2.1.6-2.6 1.6-1.1 2-.3 4.9.8 6.4.5.8 1.1 1.6 1.9 1.6.8 0 1-.5 2-.5s1.2.5 2 .5c.8 0 1.4-.8 1.9-1.5.6-.9.8-1.7.8-1.8 0 0-1.4-.6-1.4-2.6zM9.6 3.7c.4-.5.7-1.2.6-1.9-.6 0-1.3.4-1.8.9-.4.4-.7 1.1-.6 1.8.7.1 1.4-.3 1.8-.8z" /></svg>macOS</h2>
			<div class="download-cards">
				<a class="download-card" href={current.zip}>
					<img src="/favicon.svg" alt="" width="32" height="32" />
					<span>
						<strong>Apple silicon</strong>
						<small>.zip · macOS 14 or later</small>
					</span>
				</a>
			</div>
			<p class="download-note">Signed and notarized. It updates itself.</p>
		</section>

		<section class="download-section">
			<h2><svg class="section-icon" viewBox="0 0 16 16" aria-hidden="true"><path d="M3 4.5l3.5 3.5L3 11.5M8.5 12h5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" /></svg>Homebrew</h2>
			<div class="command">
				<code>{brew}</code>
				<CopyButton text={brew} label="Copy the Homebrew command" copiedMessage="Copied the Homebrew command." />
			</div>
		</section>

	{/if}

	<section class="download-section">
		<h2><svg class="section-icon" viewBox="0 0 16 16" aria-hidden="true"><path d="M3 4.5l3.5 3.5L3 11.5M8.5 12h5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" /></svg>Nix</h2>
		<div class="command">
			<code>{nixInstall}</code>
			<CopyButton text={nixInstall} label="Copy the Nix command" copiedMessage="Copied the Nix command." />
		</div>
		<p class="download-note">Builds main. A Nix install never updates itself; upgrade it through Nix.</p>
	</section>

	<p class="download-all">All builds are on the <a href={releasesUrl}>GitHub releases page <span class="arrow">↗</span></a></p>
</main>
<Footer />
