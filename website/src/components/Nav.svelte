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
			<a href="/download/">Download</a>
			<a href={repo} class="nav-gh">
				<span class="gh icon-[simple-icons--github]" aria-hidden="true"></span>
				<span>GitHub</span>
			</a>
		</nav>
	</div>
</header>
