export const repo = 'https://github.com/x-ryoppippi/dupin';
export const docsUrl = `${repo}/tree/main/docs`;
export const releasesUrl = `${repo}/releases`;
export const sponsorUrl = 'https://github.com/sponsors/ryoppippi';
export const siteUrl = 'https://dupin.app';
export const siteName = 'Dupin';

export const channels = {
	stable: {
		label: 'Stable',
		cask: 'x-ryoppippi/tap/dupin',
		zip: `${repo}/releases/latest/download/Dupin.zip`,
	},
	nightly: {
		label: 'Nightly',
		cask: 'x-ryoppippi/tap/dupin@nightly',
		zip: `${repo}/releases/download/nightly/Dupin.zip`,
		release: `${repo}/releases/tag/nightly`,
	},
} as const;

export const nixInstall = 'nix profile install github:x-ryoppippi/dupin';

export const setupPrompt = `Install Dupin, a launcher for Apple silicon Macs, on this Mac and make ⌘Space open it.

1. Check that this is an Apple silicon Mac on macOS 14 or later.
2. Free ⌘Space: Spotlight's shortcut is under System Settings → Keyboard → Keyboard Shortcuts… → Spotlight → "Show Spotlight search". Also check whether Raycast, Alfred or another launcher holds ⌘Space. Tell me exactly what to untick if you cannot change it yourself.
3. Install it with Homebrew: \`brew install --cask ${channels.stable.cask}\`, then \`open -a Dupin\`. If that cask is not published yet, use \`brew install --cask ${channels.nightly.cask}\` instead. Without Homebrew, run \`${nixInstall}\` and \`open ~/.nix-profile/Applications/Dupin.app\`, asking me before installing Nix.
4. Ask me to press ⌘Space. If the panel does not show, read ~/Library/Logs/Dupin/dupin.log and tell me why.

Settings are described at ${repo}/blob/main/docs/configuration.md
`;
