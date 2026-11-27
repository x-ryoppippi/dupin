<script lang="ts">
	// A recording of the app from scripts/docs-images/images.toml. It plays, muted and looping,
	// only while it is on screen and `active`; under reduced motion it stays on its poster until
	// played. An inactive one is paused and rewound, so it starts from its first frame when it
	// is shown, and `onready` says when that frame can be drawn.
	type Props = {
		src: string;
		poster: string;
		width: number;
		height: number;
		label: string;
		preload?: 'auto' | 'metadata' | 'none';
		active?: boolean;
		onready?: () => void;
		class?: string;
	};

	let {
		src,
		poster,
		width,
		height,
		label,
		preload = 'metadata',
		active = true,
		onready,
		class: className,
	}: Props = $props();

	let video = $state<HTMLVideoElement>();
	let paused = $state(true);
	let seen = $state(false);
	let reduced = $state(false);
	// Paused by hand, so scrolling it back into view leaves it paused.
	let chosen = false;

	$effect(() => {
		if (!video) return;
		const element = video;
		const motion = window.matchMedia('(prefers-reduced-motion: reduce)');
		reduced = motion.matches;
		const change = () => (reduced = motion.matches);
		const observer = new IntersectionObserver(([entry]) => (seen = entry?.isIntersecting ?? false));
		observer.observe(element);
		motion.addEventListener('change', change);
		if (element.readyState >= 2) onready?.();
		return () => {
			observer.disconnect();
			motion.removeEventListener('change', change);
		};
	});

	$effect(() => {
		if (!video) return;
		if (active && seen && !chosen && !reduced) void video.play().catch(() => undefined);
		else video.pause();
		if (!active && video.currentTime > 0) video.currentTime = 0;
	});

	const toggle = () => {
		chosen = !paused;
		if (paused) void video?.play();
		else video?.pause();
	};
</script>

<figure class={['clip', paused && 'is-paused', className]}>
	<video
		bind:this={video}
		{poster}
		{width}
		{height}
		{preload}
		muted
		loop
		playsinline
		aria-label={label}
		onloadeddata={() => onready?.()}
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
