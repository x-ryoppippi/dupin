<script lang="ts">
	import { repo } from '../site.ts';
	import GitHubIcon from './GitHubIcon.svelte';
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
			<a href="/download">Download</a>
			<a href="/#install">Install</a>
			<a href={repo} class="nav-gh">
				<GitHubIcon />
				<span>GitHub</span>
			</a>
		</nav>
	</div>
</header>
