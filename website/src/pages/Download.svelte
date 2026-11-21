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
			<p class="download-note">Signed and notarized. It updates itself.</p>
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
