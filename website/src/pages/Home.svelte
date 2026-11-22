<script lang="ts">
	import CopyButton from '../components/CopyButton.svelte';
	import Footer from '../components/Footer.svelte';
	import Nav from '../components/Nav.svelte';
	import PanelDemo from '../components/PanelDemo.svelte';
	import { channels, downloadChannel, repo, setupPrompt, stableReleased } from '../site.ts';

</script>

<Nav />
<main>
	<section class="hero">
		<div class="hero-grid" aria-hidden="true"></div>
		<div class="container hero-inner">
			<h1 class="display hero-title rise" style="--d: 0ms">Press ⌘Space. It's there.</h1>
			<p class="hero-sub rise" style="--d: 100ms">
				A fast, simple and beautiful app launcher for your mac.<br />
				The simplicity of Alfred, with Raycast-style extensions on the way.
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
					<a href="/download/">All downloads</a>
				</p>
			</div>
		</div>
		<div class="container rise" style="--d: 300ms">
			<PanelDemo />
			<ul class="traits">
				<li>
					<strong>~20 ms</strong>
					<span>from ⌘Space to the panel on screen. It is drawn before you ask for it.</span>
				</li>
				<li>
					<strong>Written in Rust</strong>
					<span>Native and fast, drawn on Liquid Glass. No web view, no Electron.</span>
				</li>
				<li>
					<strong>Open source</strong>
					<span>GPL-3.0. Read it, build it with Nix, change it.</span>
				</li>
			</ul>
		</div>
	</section>
</main>
<Footer />
