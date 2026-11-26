<script lang="ts">
	import CopyButton from '../components/CopyButton.svelte';
	import DesignChoices from '../components/docs/DesignChoices.svelte';
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
			<a class="pronunciation rise" style="--d: 0ms" href="/docs/name/">
				Dupin <span lang="fr">/dy.pɛ̃/</span>
			</a>
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
	<section class="philosophy">
		<div class="container">
			<p class="principle">
				A launcher should be the doorway to your desktop, not a replacement for it.
			</p>
			<DesignChoices />
			<a class="link-arrow" href="/docs/philosophy/">Read the philosophy <span class="arrow">→</span></a>
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

	.btn-primary {
		display: inline-flex;
		align-items: center;
		gap: 10px;
		padding: 14px 22px;
		border: 0;
		border-radius: 10px;
		background: var(--accent-btn);
		color: #fff;
		font: inherit;
		font-size: 15px;
		font-weight: 600;
		cursor: pointer;
		transition:
			background 0.18s ease,
			transform 0.18s ease;
	}

	.btn-primary:hover {
		background: var(--accent-btn-hover);
		transform: translateY(-1px);
	}

	.btn-primary .btn-icon {
		width: 16px;
		height: 16px;
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

	.philosophy {
		padding: 0 0 96px;
	}

	.philosophy .container {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 32px;
		max-width: 1080px;
	}

	.philosophy :global(.doc-choices) {
		grid-template-columns: repeat(4, 1fr);
		width: 100%;
	}

	.principle {
		margin: 0;
		max-width: 28ch;
		font-size: clamp(22px, 3vw, 32px);
		font-weight: 600;
		line-height: 1.25;
		letter-spacing: -0.02em;
		text-align: center;
		text-wrap: balance;
	}

	@media (max-width: 1000px) {
		.philosophy :global(.doc-choices) {
			grid-template-columns: repeat(2, 1fr);
		}
	}

	@media (max-width: 820px) {
		.philosophy :global(.doc-choices) {
			grid-template-columns: 1fr;
		}

		.traits {
			grid-template-columns: 1fr;
			gap: 20px;
		}
	}
</style>
