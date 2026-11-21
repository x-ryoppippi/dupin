---
title: Dupin
description: A lightweight, resident launcher for Apple silicon Macs. Press ⌘Space and the panel is already there.
---

## Install

Dupin runs on Apple silicon Macs and installs with [Nix](https://nixos.org/download/).

### 1. Free ⌘Space

macOS delivers a chord to only one app, so take ⌘Space back first.

- System Settings → Keyboard → Keyboard Shortcuts… → Spotlight → untick "Show Spotlight search".
- Remove ⌘Space from any other launcher that holds it, such as Raycast or Alfred.

### 2. Install and open

```sh
nix profile install github:x-ryoppippi/dupin
open ~/.nix-profile/Applications/Dupin.app
```

### Or build from source

```sh
git clone https://github.com/x-ryoppippi/dupin
cd dupin
nix build .#dupin
open result/Applications/Dupin.app
```

Dupin has no Dock icon and no menu bar item. Press the hotkey to show the panel, and ⌘, in it to change the hotkey and the rest of the [settings](https://github.com/x-ryoppippi/dupin/blob/main/docs/configuration.md).
