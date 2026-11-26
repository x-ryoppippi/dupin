import {
	channels,
	docsUrl,
	downloadChannel,
	freeHotkeySteps,
	nixInstall,
	releasesUrl,
	repo,
	setupPrompt,
	siteDescription,
	siteName,
	siteUrl,
	stableReleased,
} from './site.ts';

// Markdown twins of the pages, for agents and other readers that prefer text. The home page's
// follows the llms.txt convention (a title, a one-line summary, then sections) and is also /llms.txt.
export const homeMarkdown = `# ${siteName}

> ${siteDescription}

A launcher should be the doorway to your desktop, not a replacement for it. ${siteName} is pronounced /dy.pɛ̃/ (French). Philosophy: ${siteUrl}/docs/philosophy/

${siteName} is a launcher for Apple silicon Macs on macOS 14 or later. It stays resident and draws its panel ahead of time, so ⌘Space shows it at once. It fuzzy-searches apps by any name macOS knows them by, searches files with Spotlight after Space on an empty query, and searches inside files after \`in\` and Space. Under 3 ms pass from a keystroke to the frame that shows its results (95th percentile of the main search, measured in CI on every change).

## What it does

- Keywords: a keyword and Space enters a command, such as \`cal\` for the calendar, \`d\` for the dictionary, \`clip\` for clipboard history or \`gh\` for a web search. \`?\` lists every one. Aliases and global shortcuts reach commands, apps, web searches and files.
- Clipboard history, images and files included, and snippets that paste with the date, time or clipboard filled in.
- Calculations and unit or currency conversions in the main search, emoji, window management, system commands, Restart and Update Dupin, and your own script commands.
- Snapshots of a calculation, search or extension view, and an MCP server (\`dupin mcp\`) for agents.
- Extensions: TypeScript or JavaScript packages installed from Git, run in a sandbox with the capabilities you grant, and tested against fixtures with \`dupin ext test\`.
- Backdrop shaders: Flow, built in, and example WGSL shaders behind the panel, or your own; off by default.
- No account: what Dupin keeps stays on your Mac, encrypted, and can sync between your Macs through a folder you already sync. Official releases send one anonymous usage ping a day, which one switch turns off: ${siteUrl}/docs/features/telemetry/

## Install

A coding agent can set ${siteName} up from this prompt:

\`\`\`text
${setupPrompt.trimEnd()}
\`\`\`

By hand:

- Homebrew: \`brew install --cask ${channels[downloadChannel].cask}\`, then \`open -a Dupin\`. ${stableReleased ? `Use \`${channels.nightly.cask}\` for builds of \`main\`. ` : 'This is the nightly build until the first stable release. '}These builds update themselves.
- Nix: \`${nixInstall}\`, then \`open ~/.nix-profile/Applications/Dupin.app\`. A Nix build does not update itself.

Free ⌘Space first, since macOS gives a shortcut to only one app:

${freeHotkeySteps.map((step, i) => `${i + 1}. ${step}`).join('\n')}

## Links

- [Homepage](${siteUrl}/)
- [Download](${siteUrl}/download/)
- [Source on GitHub](${repo})
- [Documentation](${siteUrl}${docsUrl})
- [Releases](${releasesUrl})
`;

export const downloadMarkdown = `# Download ${siteName}

${siteName} runs on Apple silicon Macs with macOS 14 or later.

## Stable

${
	stableReleased
		? `- App: [Dupin.dmg](${channels.stable.dmg}), signed and notarized; moved to Applications, it updates itself.
- Homebrew: \`brew install --cask ${channels.stable.cask}\``
		: 'Not released yet. Install the nightly build until the first stable release is out.'
}

## Nightly

Built daily from \`main\`.

- App: [Dupin.dmg](${channels.nightly.dmg}), signed and notarized; moved to Applications, it updates itself.
- Homebrew: \`brew install --cask ${channels.nightly.cask}\`

## Nix

\`${nixInstall}\` builds \`main\`. A Nix install does not update itself; upgrade it through Nix.

All builds are on the [GitHub releases page](${releasesUrl}). The signed builds send [one anonymous usage ping a day](${siteUrl}/docs/features/telemetry/), which one switch turns off; a Nix build sends none.

## Free ⌘Space first

macOS gives a shortcut to only one app, so take ⌘Space back before you open Dupin.

${freeHotkeySteps.map((step, i) => `${i + 1}. ${step}`).join('\n')}
`;
