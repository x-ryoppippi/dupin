import { Kbd } from '../components/Kbd.tsx';

const modes = [
	{
		keys: ['Type'],
		title: 'Apps',
		body: 'Fuzzy-search every app in /Applications, by any name macOS knows it by.',
	},
	{
		keys: ['␣'],
		title: 'Files',
		body: 'Space on an empty query searches file and display names with Spotlight.',
	},
	{
		keys: ['in', '␣'],
		title: 'In files',
		body: 'Search the text Spotlight indexed from inside your files.',
	},
	{ keys: ['⌘', '↩'], title: 'Reveal', body: 'Open the result, or show it in Finder instead.' },
];

export function Modes() {
	return (
		<section class="band">
			<div class="container">
				<p class="eyebrow">Four modes, no menus</p>
				<div class="strip">
					{modes.map((m) => (
						<div class="strip-cell">
							<div class="strip-keys">
								{m.keys.map((k) => (
									<Kbd>{k}</Kbd>
								))}
							</div>
							<h3>{m.title}</h3>
							<p>{m.body}</p>
						</div>
					))}
				</div>
			</div>
		</section>
	);
}
