<script lang="ts">
	import CopyButton from '../components/CopyButton.svelte';
	import Footer from '../components/Footer.svelte';
	import Nav from '../components/Nav.svelte';
	import PanelDemo from '../components/PanelDemo.svelte';
	import { channels, downloadChannel, repo, setupPrompt, stableReleased } from '../site.ts';

	let { installHtml }: { installHtml: string } = $props();
</script>

<Nav />
<main>
	<section class="hero">
		<div class="hero-grid" aria-hidden="true"></div>
		<div class="container hero-inner">
			<h1 class="display hero-title rise" style="--d: 0ms">Press ⌘Space. It's there.</h1>
			<p class="hero-sub rise" style="--d: 100ms">
				A resident launcher for Apple silicon Macs. Dupin draws its panel before you ask for it, so the
				hotkey only has to show it: apps, files and what's inside them, on Liquid Glass.
			</p>
			<div class="cta rise" style="--d: 200ms">
				<div class="cta-buttons">
					<a class="btn-primary" href={channels[downloadChannel].dmg}>
						<span class="btn-icon icon-[simple-icons--apple]" aria-hidden="true"></span>
						Download for macOS
					</a>
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
					{#if !stableReleased}Nightly build until the first release ·{/if}
					<a href="/download">All downloads</a> · <a href="#install">Install by hand</a>
				</p>
			</div>
		</div>
		<div class="container rise" style="--d: 300ms">
			<PanelDemo />
		</div>
	</section>

	<section class="band install" id="install">
		<div class="container install-inner">
			<div class="prose">{@html installHtml}</div>
		</div>
	</section>
</main>
<Footer />
