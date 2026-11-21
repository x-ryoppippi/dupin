import {
	channels,
	docsUrl,
	nixInstall,
	releasesUrl,
	repo,
	setupPrompt,
	siteDescription,
	siteName,
	siteUrl,
} from './site.ts';

// Markdown twins of the pages, for agents and other readers that prefer text. The home page's
// follows the llms.txt convention (a title, a one-line summary, then sections) and is also /llms.txt.
export const homeMarkdown = `# ${siteName}

> ${siteDescription}

${siteName} is a launcher for Apple silicon Macs on macOS 14 or later. It stays resident and draws its panel ahead of time, so ⌘Space shows it at once. It fuzzy-searches apps by any name macOS knows them by, searches files with Spotlight after Space on an empty query, and searches inside files after \`in\` and Space.

## Install

A coding agent can set ${siteName} up from this prompt:

\`\`\`text
${setupPrompt.trimEnd()}
\`\`\`

By hand:

- Homebrew: \`brew install --cask ${channels.stable.cask}\` (or \`${channels.nightly.cask}\` for builds of \`main\`), then \`open -a Dupin\`. These builds update themselves.
- Nix: \`${nixInstall}\`, then \`open ~/.nix-profile/Applications/Dupin.app\`. A Nix build does not update itself.

## Links

- [Homepage](${siteUrl}/)
- [Download](${siteUrl}/download/)
- [Source on GitHub](${repo})
- [Documentation](${docsUrl})
- [Releases](${releasesUrl})
`;

export const downloadMarkdown = `# Download ${siteName}

${siteName} runs on Apple silicon Macs with macOS 14 or later.

## Stable

- App: [Dupin.zip](${channels.stable.zip}), signed and notarized; it updates itself.
- Homebrew: \`brew install --cask ${channels.stable.cask}\`

## Nightly

Built daily from \`main\`.

- App: [Dupin.zip](${channels.nightly.zip})
- Homebrew: \`brew install --cask ${channels.nightly.cask}\`

## Nix

\`${nixInstall}\` builds \`main\`. A Nix install does not update itself; upgrade it through Nix.

All builds are on the [GitHub releases page](${releasesUrl}).
`;
