<script lang="ts">
	import CopyButton from '../components/CopyButton.svelte';
	import Footer from '../components/Footer.svelte';
	import GitHubIcon from '../components/GitHubIcon.svelte';
	import Nav from '../components/Nav.svelte';
	import PanelDemo from '../components/PanelDemo.svelte';
	import { repo, setupPrompt } from '../site.ts';

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
				<CopyButton
					class="btn-primary"
					text={setupPrompt}
					label="Copy a setup prompt for your coding agent"
					copiedMessage="Copied the setup prompt. Paste it into your coding agent."
				>
					{#snippet children()}
						<svg class="btn-icon" viewBox="0 0 16 16" aria-hidden="true">
							<rect x="5" y="5" width="8.5" height="8.5" rx="2" />
							<path d="M10.5 3.2A1.7 1.7 0 0 0 9 2.5H4.2a1.7 1.7 0 0 0-1.7 1.7V9a1.7 1.7 0 0 0 .7 1.5" />
						</svg>
						Copy setup prompt
					{/snippet}
					{#snippet copiedChildren()}
						<svg class="btn-icon" viewBox="0 0 16 16" aria-hidden="true"><path d="M3.5 8.5l3 3 6-7" /></svg>
						Copied. Paste it into your agent
					{/snippet}
				</CopyButton>
				<a class="link-arrow" href={repo}>
					<GitHubIcon />
					Read the source (it's GPL) <span class="arrow">↗</span>
				</a>
				<p class="cta-more">
					Rather not use an agent? <a href="/download">Download</a> · <a href="#install">Install by hand</a>
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
