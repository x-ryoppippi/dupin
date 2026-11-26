<script lang="ts">
	import type { Attachment } from 'svelte/attachments';

	// A recording of the app from scripts/docs-images/images.toml. It plays, muted and looping,
	// only while it is on screen; under reduced motion it stays on its poster until played.
	type Props = {
		src: string;
		poster: string;
		width: number;
		height: number;
		label: string;
		preload?: 'auto' | 'metadata' | 'none';
		class?: string;
	};

	let { src, poster, width, height, label, preload = 'metadata', class: className }: Props = $props();

	let video: HTMLVideoElement | undefined;
	let paused = $state(true);
	// Paused by hand, so scrolling it back into view leaves it paused.
	let chosen = false;

	const autoplay: Attachment<HTMLVideoElement> = (element) => {
		video = element;
		const motion = window.matchMedia('(prefers-reduced-motion: reduce)');
		let seen = false;
		const sync = () => {
			if (seen && !chosen && !motion.matches) void element.play().catch(() => undefined);
			else if (!seen || motion.matches) element.pause();
		};
		const observer = new IntersectionObserver(([entry]) => {
			seen = entry?.isIntersecting ?? false;
			sync();
		});
		observer.observe(element);
		motion.addEventListener('change', sync);
		return () => {
			observer.disconnect();
			motion.removeEventListener('change', sync);
		};
	};

	const toggle = () => {
		chosen = !paused;
		if (paused) void video?.play();
		else video?.pause();
	};
</script>

<figure class={['clip', paused && 'is-paused', className]}>
	<video
		{@attach autoplay}
		{poster}
		{width}
		{height}
		{preload}
		muted
		loop
		playsinline
		aria-label={label}
		onplay={() => (paused = false)}
		onpause={() => (paused = true)}
	>
		<source {src} type="video/mp4" />
	</video>
	<button type="button" class="toggle" aria-label={paused ? 'Play the recording' : 'Pause the recording'} onclick={toggle}>
		<span class={paused ? 'icon-[ph--play-fill]' : 'icon-[ph--pause-fill]'} aria-hidden="true"></span>
	</button>
</figure>

<style>
	.clip {
		position: relative;
		margin: 0;
	}

	video {
		display: block;
		width: 100%;
		height: auto;
	}

	.toggle {
		position: absolute;
		right: 8px;
		bottom: 8px;
		display: grid;
		place-items: center;
		width: 28px;
		height: 28px;
		padding: 0;
		border: 1px solid var(--border-strong);
		border-radius: 999px;
		background: color-mix(in srgb, var(--bg) 70%, transparent);
		color: var(--fg-muted);
		cursor: pointer;
		opacity: 0;
		transition: opacity 0.18s ease;
	}

	.clip:hover .toggle,
	.toggle:focus-visible,
	.clip.is-paused .toggle {
		opacity: 1;
	}

	.toggle span {
		width: 12px;
		height: 12px;
	}
</style>
