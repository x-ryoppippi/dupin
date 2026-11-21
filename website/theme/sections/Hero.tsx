import { InstallCard } from '../components/InstallCard.tsx';
import { PanelDemo } from '../components/PanelDemo.tsx';

export function Hero() {
	return (
		<section class="hero">
			<div class="hero-grid" aria-hidden="true" />
			<div class="container hero-inner">
				<h1 class="display hero-title" data-rise style="--d: 0ms">
					Press ⌘Space. It's there.
				</h1>
				<p class="hero-sub" data-rise style="--d: 100ms">
					A resident launcher for Apple silicon Macs. Dupin draws its panel before you ask for it,
					so the hotkey only has to show it: apps, files and what's inside them, on Liquid Glass.
				</p>
				<InstallCard />
			</div>
			<div class="container">
				<PanelDemo />
			</div>
		</section>
	);
}
