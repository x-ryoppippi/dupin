<script lang="ts">
	import version from '../../../VERSION?raw';
	import CopyButton from '../components/CopyButton.svelte';
	import Footer from '../components/Footer.svelte';
	import Nav from '../components/Nav.svelte';
	import { channels, nixInstall, releasesUrl, repo } from '../site.ts';

	const stableVersion = version.trim();
	let channel = $state<keyof typeof channels>('stable');
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
			{#if channel === 'stable'}
				<a href={`${repo}/releases/tag/v${stableVersion}`}>v{stableVersion} <span class="arrow">↗</span></a>
			{:else}
				<a href={channels.nightly.release}>Built daily from main <span class="arrow">↗</span></a>
			{/if}
		</p>
	</header>

	<section class="download-section">
		<h2><span class="section-icon icon-[simple-icons--apple]" aria-hidden="true"></span>macOS</h2>
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
		<h2><span class="section-icon icon-[simple-icons--homebrew]" aria-hidden="true"></span>Homebrew</h2>
		<div class="command">
			<code>{brew}</code>
			<CopyButton text={brew} label="Copy the Homebrew command" copiedMessage="Copied the Homebrew command." />
		</div>
	</section>

	<section class="download-section">
		<h2><span class="section-icon icon-[simple-icons--nixos]" aria-hidden="true"></span>Nix</h2>
		<div class="command">
			<code>{nixInstall}</code>
			<CopyButton text={nixInstall} label="Copy the Nix command" copiedMessage="Copied the Nix command." />
		</div>
		<p class="download-note">Builds main. A Nix install never updates itself; upgrade it through Nix.</p>
	</section>

	<p class="download-all">All builds are on the <a href={releasesUrl}>GitHub releases page <span class="arrow">↗</span></a></p>
</main>
<Footer />
