export function items() {
	return {
		sections: [
			{
				title: 'Projects',
				items: [
					{
						id: 'dupin',
						title: 'Dupin',
						subtitle: 'github.com/x-ryoppippi/dupin',
						icon: 'icons/mark.svg',
						url: 'https://github.com/x-ryoppippi/dupin',
					},
					{
						id: 'rune',
						title: 'Rune',
						subtitle: 'github.com/anomalyco/rune',
						icon: 'icons/mark.svg',
						url: 'https://github.com/anomalyco/rune',
						detail: {
							markdown:
								'## Rune\n\nThe **UI toolkit** Dupin draws with.\n\n- Retained panels\n- `launcher_panel`\n\n[Repository](https://github.com/anomalyco/rune)',
							metadata: [
								{ label: 'Language', text: 'Rust' },
								{ label: 'License', text: 'MIT' },
							],
						},
					},
				],
			},
			{
				title: 'Reading',
				items: [
					{
						id: 'book',
						title: 'The Rust Book',
						subtitle: 'doc.rust-lang.org/book',
						icon: 'icons/mark.svg',
						url: 'https://doc.rust-lang.org/book/',
					},
				],
			},
		],
	};
}
