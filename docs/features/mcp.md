# MCP server

`dupin mcp` serves the [Model Context Protocol](https://modelcontextprotocol.io) over stdio, so an agent such as Claude Code can read Dupin's configuration and use its panel. Dupin itself has no chat or model features: the agent runs elsewhere, and Dupin is a tool it calls.

The server is a thin client of the same local control socket as the [`dupin` commands](../../README.md#local-cli). It starts no window and holds no state of its own, and each agent session starts its own `dupin mcp` process. Tools that act on the panel need the resident launcher running; the configuration tools work without it.

## Setting it up in Claude Code

Add Dupin to a project's `.mcp.json`:

```json
{
	"mcpServers": {
		"dupin": {
			"command": "/Applications/Dupin.app/Contents/MacOS/dupin",
			"args": ["mcp"]
		}
	}
}
```

With the Nix package or the home-manager module, `dupin` is on `PATH`, so `"command": "dupin"` works too. To add it for every project instead, run `claude mcp add --scope user dupin -- dupin mcp`.

## Tools

| Tool                | Needs the launcher | What it does                                                                                                                                                               |
| ------------------- | ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `read_config`       | No                 | Returns both configuration files and the `lockedKeys` from a read-only `config.toml`.                                                                                      |
| `validate_config`   | No                 | Checks a whole `config.toml` source as Dupin would load it, hotkey included, and writes nothing.                                                                           |
| `get_active_config` | Yes                | Returns the values the launcher has applied, as `dupin config get` prints them.                                                                                            |
| `open_panel`        | Yes                | Shows the panel, optionally with a `query` typed in, as `dupin open --query` does. Nothing is activated, and Enter does nothing for half a second after the panel appears. |
| `pick`              | Yes                | Shows `values` (1 to 1000 JSON values) in the panel and waits up to five minutes for the user to choose one, as `dupin pick` does.                                         |

`read_config` reports the base's `path`, `exists`, `readOnly` and TOML `source` at the top level. The `overlay` has the same fields plus `active`, which is true when the base is read-only. `lockedKeys` is empty for a writable base and null if the read-only base has invalid TOML syntax.

`validate_config` answers `{"valid": true, "config": {...}}` with the resulting values, or `{"valid": false, "problem": "line 2, column 1: ..."}`. `pick` answers `{"value": ...}` with the chosen value unchanged.

No tool writes configuration, runs a script or opens a file. Those have effects outside Dupin, and will ask for confirmation in the panel before they run ([#34](https://github.com/x-ryoppippi/dupin/issues/34)).

## Errors

A tool that fails, such as `open_panel` while Dupin isn't running ("Dupin is not running"), a `pick` the user dismisses ("Cancelled") or an argument it doesn't accept, returns a result with `isError: true` and the reason as text, so the agent can correct itself. Protocol mistakes are JSON-RPC errors: invalid JSON (`-32700`), a malformed request, a batch or a message over 512 KiB (`-32600`), an unknown method (`-32601`), and an unknown tool or malformed `tools/call` parameters (`-32602`).

Tool calls run concurrently, so a `pick` waiting on the user does not hold up other requests. Cancelling a call (`notifications/cancelled`) closes its picker and the call gets no response. When the client closes stdin, any picker still waiting closes too.

## Protocol

Messages are JSON-RPC 2.0, one per line, on stdin and stdout; nothing else is written to stdout. The server implements `initialize`, `ping`, `tools/list` and `tools/call`, and accepts `notifications/initialized` and `notifications/cancelled`. It answers `initialize` with the client's protocol version when it is one of 2025-11-25, 2025-06-18, 2025-03-26 or 2024-11-05, and with 2025-11-25 otherwise.
