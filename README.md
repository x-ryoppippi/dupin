<div align="center">
    <img src="./assets/logo/dupin.svg" alt="Dupin logo" width="112" height="112">
    <h1>Dupin</h1>
    <p><strong>type a keyword, reach anything</strong></p>
    <p>A light, always-ready Mac launcher: Alfred's successor, with extensions your agent can write.</p>
    <p>
        <a href="https://dupin.app">Website</a> ·
        <a href="https://dupin.app/docs/">Docs</a> ·
        <a href="https://dupin.app/download/">Download</a> ·
        <a href="https://github.com/x-ryoppippi/dupin/releases">Releases</a>
    </p>
    <p>
        <a href="https://github.com/x-ryoppippi/dupin/actions/workflows/ci.yml"><img src="https://github.com/x-ryoppippi/dupin/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI"></a>
        <a href="./LICENSE"><img src="https://img.shields.io/github/license/x-ryoppippi/dupin" alt="Licence: GPL-3.0-or-later"></a>
    </p>
    <picture>
        <source media="(prefers-color-scheme: dark)" srcset="./docs/images/keywords/list-dark.png">
        <img src="./docs/images/keywords/list-light.png" alt="The Dupin panel with ? typed, listing commands and their keywords: cal for Calendar, clip for Clipboard History, d and define for Dictionary, in for File Contents, Space for Files, maps for Apple Maps." width="560">
    </picture>
</div>

## Why Dupin

Dupin is what Alfred was meant to be: simple and extensible.

- **Built for searching.** Type a keyword, reach anything, at once: apps, files, `cal` for your calendar, `d` for the dictionary. Dupin stays resident, so the panel is there the moment you press the hotkey.
- **Simple.** The launcher stays a launcher.
- **Extensible.** Extensions are plain TypeScript that you or your agent can write, test and install from Git, each sandboxed with only the permissions you grant.
- **Mac-native**, on Apple silicon with macOS 14 or later.

More in [Philosophy](./docs/philosophy.md). Named after C. Auguste Dupin, Poe's detective, who came before Sherlock, Watson and Alfred ([the name](./docs/name.md)).

## Install

```sh
brew install --cask x-ryoppippi/tap/dupin@nightly
```

The nightly cask builds `main` and updates itself; `x-ryoppippi/tap/dupin` is the stable channel from the first stable release. With Nix, run `nix profile install github:x-ryoppippi/dupin` or use the [home-manager module](./docs/configuration.md#nix).

Free ⌘Space first: turn off Spotlight's shortcut and unassign it in any other launcher ([how](./docs/features/hotkeys.md#freeing-the-hotkey)).

<details>
<summary>Or let a coding agent set it up</summary>

```text
Install Dupin, a launcher for Apple silicon Macs, on this Mac and make ⌘Space open it.

1. Check that this is an Apple silicon Mac on macOS 14 or later.
2. Free ⌘Space: Spotlight's shortcut is under System Settings → Keyboard → Keyboard Shortcuts… → Spotlight → "Show Spotlight search". Also check whether Raycast, Alfred or another launcher holds ⌘Space. Tell me exactly what to untick if you cannot change it yourself.
3. Install it with Homebrew: `brew install --cask x-ryoppippi/tap/dupin`, then `open -a Dupin`. If that cask is not published yet, use `brew install --cask x-ryoppippi/tap/dupin@nightly` instead. Without Homebrew, run `nix profile install github:x-ryoppippi/dupin` and `open ~/.nix-profile/Applications/Dupin.app`, asking me before installing Nix.
4. Ask me to press ⌘Space. If the panel does not show, read ~/Library/Logs/Dupin/dupin.log and tell me why.

Settings are described at https://dupin.app/docs/configuration/
```

</details>

## First steps

| Type                       | To                                   |
| -------------------------- | ------------------------------------ |
| ⌘Space                     | Show or hide the panel               |
| A few letters of an app, ↩ | Open it                              |
| Space, then a name         | Search files with Spotlight          |
| `cal` or `d`, then Space   | Search your calendar, look up a word |
| `?`                        | List every command and its keyword   |
| ⌘K                         | Show actions for the selected result |

The rest is in the [docs](https://dupin.app/docs/), starting with the [tutorial](./docs/tutorial.md).

<table>
    <tr>
        <td width="33%"><picture><source media="(prefers-color-scheme: dark)" srcset="./docs/images/clipboard/history-dark.png"><img src="./docs/images/clipboard/history-light.png" alt="Clipboard history showing a copied image with its size and the app it came from."></picture></td>
        <td width="33%"><picture><source media="(prefers-color-scheme: dark)" srcset="./docs/images/calendar/upcoming-dark.png"><img src="./docs/images/calendar/upcoming-light.png" alt="The calendar keyword listing today's and upcoming events."></picture></td>
        <td width="33%"><picture><source media="(prefers-color-scheme: dark)" srcset="./docs/images/extensions/form-dark.png"><img src="./docs/images/extensions/form-light.png" alt="A form an extension shows in the panel, with text, checkbox and choice fields."></picture></td>
    </tr>
    <tr>
        <td align="center"><a href="./docs/features/clipboard.md">Clipboard history</a></td>
        <td align="center"><a href="./docs/features/calendar.md">Calendar</a></td>
        <td align="center"><a href="./docs/features/extensions.md">Your own extensions</a></td>
    </tr>
</table>

## Links

[Documentation](./docs/README.md) · [Writing extensions](./docs/features/extensions.md) · [Command line](./docs/features/cli.md) · [Development](./docs/development.md) · [Contributing](./CONTRIBUTING.md)

Dupin is licensed under [GPL-3.0-or-later](./LICENSE); contributions need the [Contributor License Agreement](./CLA.md).

## Sponsors

<p align="center">
    <a href="https://coderabbit.link/ryoppippi"><picture><source media="(prefers-color-scheme: dark)" srcset="./docs/public/coderabbit-logo-dark.svg"><img src="./docs/public/coderabbit-logo.svg" alt="CodeRabbit" width="200"></picture></a>
    &nbsp;&nbsp;&nbsp;
    <a href="https://blacksmith.sh"><picture><source media="(prefers-color-scheme: dark)" srcset="./docs/public/blacksmith-logo-dark.svg"><img src="./docs/public/blacksmith-logo.svg" alt="CI powered by Blacksmith" width="200"></picture></a>
</p>

<p align="center">
    <a href="https://github.com/sponsors/ryoppippi"><img src="https://sponsors.ryoppippi.com/sponsors.png" alt="GitHub Sponsors"></a>
</p>
