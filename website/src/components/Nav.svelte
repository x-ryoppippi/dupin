<script lang="ts">
	import { repo } from '../site.ts';
	import Logo from './Logo.svelte';

	let scrolled = $state(false);
</script>

<svelte:window onscroll={() => (scrolled = window.scrollY > 12)} />

<header class="nav" class:is-scrolled={scrolled}>
	<div class="nav-inner">
		<a class="brand" href="/">
			<Logo />
			<span>Dupin</span>
		</a>
		<nav class="nav-links">
			<a href="/docs/">Docs</a>
			<a href="/download/">Download</a>
			<a href={repo} class="nav-gh">
				<span class="gh icon-[simple-icons--github]" aria-hidden="true"></span>
				<span>GitHub</span>
			</a>
		</nav>
	</div>
</header>

<style>
	.nav {
		position: sticky;
		top: 0;
		z-index: 20;
		backdrop-filter: blur(18px);
		background: color-mix(in srgb, var(--bg) 75%, transparent);
		border-bottom: 1px solid transparent;
		transition: border-color 0.18s ease;
	}

	.nav.is-scrolled {
		border-bottom-color: var(--border);
	}

	.nav-inner {
		max-width: 1240px;
		margin: 0 auto;
		padding: 12px 32px;
		display: flex;
		align-items: center;
		justify-content: space-between;
	}

	.brand {
		display: inline-flex;
		align-items: center;
		gap: 10px;
		font-size: 16px;
		font-weight: 500;
		color: var(--fg-muted);
	}

	.nav-links {
		display: flex;
		align-items: center;
		gap: 24px;
		font-size: 13px;
		color: var(--fg-muted);
		letter-spacing: -0.01em;
	}

	.nav-links a {
		transition: color 0.18s ease;
	}

	.nav-links a:hover {
		color: var(--fg);
	}

	.nav-gh {
		display: inline-flex;
		align-items: center;
		gap: 6px;
	}

	.gh {
		width: 14px;
		height: 14px;
	}

	@media (max-width: 820px) {
		.nav-inner {
			padding-left: 16px;
			padding-right: 16px;
		}
	}
</style>
