---
title: Install
---

## Install

Dupin runs on Apple silicon Macs with macOS 14 or later. The setup prompt at the top of the page has a coding agent do these steps for you.

### 1. Free ⌘Space

macOS delivers a chord to only one app, so take ⌘Space back first.

- System Settings → Keyboard → Keyboard Shortcuts… → Spotlight → untick "Show Spotlight search".
- Remove ⌘Space from any other launcher that holds it, such as Raycast or Alfred.

### 2. Install and open

```sh
brew install --cask x-ryoppippi/tap/dupin@nightly
open -a Dupin
```

The build is signed and notarized, and it [updates itself](https://github.com/x-ryoppippi/dupin/blob/main/docs/configuration.md#updates). This is the nightly build of `main`; once the first stable release is out, `x-ryoppippi/tap/dupin` installs releases only. The app is also on the [download page](/download).

### Or with Nix

A Nix build never updates itself; upgrade it through Nix.

```sh
nix profile install github:x-ryoppippi/dupin
open ~/.nix-profile/Applications/Dupin.app
```

Dupin has no Dock icon and no menu bar item. Press the hotkey to show the panel, and ⌘, in it to change the hotkey and the rest of the [settings](https://github.com/x-ryoppippi/dupin/blob/main/docs/configuration.md).
