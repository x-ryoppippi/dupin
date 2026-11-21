---
title: Dupin
description: A lightweight, resident launcher for Apple silicon Macs. Press ⌘Space and the panel is already there.
---

## Install

Dupin runs on Apple silicon Macs and installs with [Nix](https://nixos.org/download/).

### Set up with an agent

Paste this into Claude Code, Codex or another coding agent.

```text
Install Dupin, a launcher for Apple silicon Macs, on this Mac and make ⌘Space open it.

1. Check that this is an Apple silicon Mac on macOS 14 or later and that Nix is installed. If Nix is missing, ask me before installing it.
2. Free ⌘Space: Spotlight's shortcut is under System Settings → Keyboard → Keyboard Shortcuts… → Spotlight → "Show Spotlight search". Also check whether Raycast, Alfred or another launcher holds ⌘Space. Tell me exactly what to untick if you cannot change it yourself.
3. Run `nix profile install github:x-ryoppippi/dupin`, then `open ~/.nix-profile/Applications/Dupin.app`.
4. Ask me to press ⌘Space. If the panel does not show, read ~/Library/Logs/Dupin/dupin.log and tell me why.

Settings are described at https://github.com/x-ryoppippi/dupin/blob/main/docs/configuration.md
```

### Or set it up yourself

#### 1. Free ⌘Space

macOS delivers a chord to only one app, so take ⌘Space back first.

- System Settings → Keyboard → Keyboard Shortcuts… → Spotlight → untick "Show Spotlight search".
- Remove ⌘Space from any other launcher that holds it, such as Raycast or Alfred.

#### 2. Install and open

```sh
nix profile install github:x-ryoppippi/dupin
open ~/.nix-profile/Applications/Dupin.app
```

Dupin has no Dock icon and no menu bar item. Press the hotkey to show the panel, and ⌘, in it to change the hotkey and the rest of the [settings](https://github.com/x-ryoppippi/dupin/blob/main/docs/configuration.md).
