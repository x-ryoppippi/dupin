<script lang="ts">
	import { docsUrl, releasesUrl, repo, sponsorUrl } from '../site.ts';
	import Logo from './Logo.svelte';
</script>

<footer class="footer">
	<div class="container footer-inner">
		<div class="footer-brand">
			<Logo width={30} height={14} />
			<span>© ryoppippi · GPL-3.0-or-later</span>
		</div>
		<nav class="footer-links">
			<a href={repo}>GitHub</a>
			<a href={docsUrl}>Docs</a>
			<a href="/download/">Download</a>
			<a href={releasesUrl}>Releases</a>
			<a href={sponsorUrl}>Sponsor</a>
		</nav>
	</div>
</footer>

<style>
	.footer {
		border-top: 1px solid var(--border);
		padding: 40px 0 32px;
	}

	.footer-inner {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 24px;
	}

	.footer-brand {
		display: flex;
		align-items: center;
		gap: 10px;
		font-size: 13px;
		color: var(--fg-dim);
	}

	.footer-links {
		display: flex;
		gap: 20px;
		font-size: 13px;
		color: var(--fg-muted);
	}

	.footer-links a:hover {
		color: var(--fg);
	}

	@media (max-width: 820px) {
		.footer-inner {
			flex-direction: column;
			align-items: flex-start;
		}
	}
</style>
