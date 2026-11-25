# Documentation

- [Using Dupin](./usage.md): keys, app search, file and content search
- [MCP server](./features/mcp.md): `dupin mcp`, for agents such as Claude Code
- [Configuration](./configuration.md): `config.toml`, the home-manager module, the hotkey and what happens when it's taken
- [Development](./development.md): building, dependency builds, formatting, headless renders, peak memory
- [Advanced composition](./features/advanced.md): `[advanced]` in `config.toml`, recomposing the built-in sources, filters and kinds
- [Extensions](./features/extensions.md): writing and installing JavaScript extensions, and their limits
- [Extension capabilities](./extension-capabilities.md): the host side of the `files` and `exec` capabilities
- [The `http` capability](./extension-http.md): the host service that will fetch for extensions, its scope checks and limits
- [Sync design](./design/sync.md): syncing Dupin between Macs through an end-to-end encrypted synced folder
- [Architecture](./architecture.md): crates, providers, the session and the panel
- [Encrypted storage](./features/storage.md): how Dupin keeps data on disk, its key and its threat model
- [Working around Rune](./rune-workarounds.md): what Dupin patches around in its UI framework
- [Releasing](./releasing.md): tagpr and CI
- [The name](./name.md): why Dupin, and the Mac launchers it descends from
