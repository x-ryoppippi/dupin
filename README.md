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
    <picture>
        <source media="(prefers-color-scheme: dark)" srcset="./docs/images/keywords/list-dark.png">
        <img src="./docs/images/keywords/list-light.png" alt="The Dupin panel with ? typed, listing commands and their keywords: cal for Calendar, clip for Clipboard History, d and define for Dictionary, in for File Contents, Space for Files, maps for Apple Maps." width="560">
    </picture>
</div>

## Why Dupin

- **Fast.** The panel is already drawn when you press the hotkey, and app search shows its results under 3 ms after each keystroke (p95, measured in CI on every change to the app).
- **Built for searching, and simple.** Type a keyword, reach anything; the launcher stays a launcher.
- **Extensible by anyone, agents included.** Extensions are plain TypeScript you write, test and install from Git, sandboxed with the permissions you grant.

## Install

```sh
brew install --cask x-ryoppippi/tap/dupin@nightly
nix profile install github:x-ryoppippi/dupin
```

Free ⌘Space first ([how](./docs/features/hotkeys.md#freeing-the-hotkey)). More ways to install, and everything else, are in the docs.

**Read the docs at [dupin.app/docs](https://dupin.app/docs/).**

---

[GPL-3.0-or-later](./LICENSE) · [Contributing](./CONTRIBUTING.md) · [Sponsor](https://github.com/sponsors/ryoppippi)

<p align="center">
    <a href="https://coderabbit.link/ryoppippi"><picture><source media="(prefers-color-scheme: dark)" srcset="./docs/public/coderabbit-logo-dark.svg"><img src="./docs/public/coderabbit-logo.svg" alt="CodeRabbit" width="140"></picture></a>
    &nbsp;&nbsp;
    <a href="https://blacksmith.sh"><picture><source media="(prefers-color-scheme: dark)" srcset="./docs/public/blacksmith-logo-dark.svg"><img src="./docs/public/blacksmith-logo.svg" alt="CI powered by Blacksmith" width="140"></picture></a>
</p>
