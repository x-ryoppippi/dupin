# Documentation

## Getting started

- [Using Dupin](./usage.md): keys, app search, file and content search
- [Tutorial](./tutorial.md): first steps, from opening an app to your own shortcuts
- [Configuration](./configuration.md): `config.toml`, the home-manager module, the hotkey and what happens when it's taken
- [Keywords](./features/keywords.md)

## About

- [Philosophy](./philosophy.md): why Dupin exists: built for searching, a launcher and nothing more, fast by rule, extensions agents can write
- [FAQ](./faq.md): short answers on installing, privacy, extensions and how Dupin compares
- [The name](./name.md): why Dupin, and the Mac launchers it descends from

## Built-in features

Native to Dupin and always there.

- [Apps](./features/apps.md)
- [Recent items](./features/recent-items.md)
- [Files](./features/files.md)
- [Folder browsing](./features/folder-browsing.md)
- [File actions](./features/file-actions.md)
- [Send to](./features/send-to.md)
- [Snapshots](./features/snapshots.md)
- [Script commands](./features/scripts.md)
- [Hotkeys](./features/hotkeys.md)
- [Search selected text](./features/selected-text.md)
- [System commands](./features/system-commands.md)
- [Settings](./features/settings.md)
- [Appearance](./features/appearance.md)
- [Permissions](./features/permissions.md)
- [Encrypted storage](./features/storage.md): how Dupin keeps data on disk, its key and its threat model
- [Sync](./features/sync.md): keeping your Macs in step through a synced folder, end-to-end encrypted
- [Updates](./features/updates.md)
- [Advanced composition](./features/advanced.md): `[advanced]` in `config.toml`, recomposing the built-in sources, filters and kinds

## Bundled extensions

Shipped with Dupin as extensions, each one yours to eject and change.

- [Calculator](./features/calculator.md)
- [Calendar](./features/calendar.md)
- [Clipboard history](./features/clipboard.md)
- [Dictionary](./features/dictionary.md)
- [Emoji & Symbols](./features/emoji.md)
- [Repositories](./features/repo.md)
- [Snippets](./features/snippets.md)
- [Web searches](./features/web-searches.md)
- [Maps](./features/maps.md)
- [Window management](./features/window-management.md)
- [Processes](./features/processes.md)
- [Ports](./features/ports.md)
- [Coding agents](./features/coding-agents.md)

## Your own extensions

Write, install and test extensions of your own.

- [Extensions](./features/extensions.md): writing and installing JavaScript extensions, and their limits

## Scripts and agents

Drive Dupin from a terminal, a script or an AI agent.

- [Command line](./features/cli.md): `dupin open`, `pick`, `config`, `ext` and headless renders
- [MCP server](./features/mcp.md): `dupin mcp`, for agents such as Claude Code

## Working on Dupin

- [Development](./development.md): building, dependency builds, formatting, headless renders, peak memory
- [Architecture](./architecture.md): crates, providers, the session and the panel
- [Extension capabilities](./extension-capabilities.md): the host side of the `files` and `exec` capabilities
- [The `http` capability](./extension-http.md): the host service that will fetch for extensions, its scope checks and limits
- [Sync design](./design/sync.md): syncing Dupin between Macs through an end-to-end encrypted synced folder
- [Working around Rune](./rune-workarounds.md): what Dupin patches around in its UI framework
- [Releasing](./releasing.md): tagpr and CI
