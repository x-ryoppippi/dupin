# MCP server

`dupin mcp` serves the [Model Context Protocol](https://modelcontextprotocol.io) over stdio, so an agent such as Claude Code can read Dupin's configuration and use its panel.

## Open it

Add `dupin mcp` to your agent's MCP servers (see [Configuration](#configuration)); the agent starts it.

## What you see

- Nothing of its own: Dupin has no chat or model features, and is a tool the agent calls.
- The panel, when the agent opens it or asks you to pick a value.

## What you can do

| Key / action                                   | What it does                                                           |
| ---------------------------------------------- | ---------------------------------------------------------------------- |
| `read_config`, `validate_config`               | Return or check the configuration files, without the launcher.         |
| `get_active_config`                            | Returns the values the launcher has applied.                           |
| `open_panel`                                   | Shows the panel, optionally with a `query` typed in.                   |
| `pick`                                         | Shows `values` in the panel and waits for you to choose one.           |
| <kbd>↩</kbd> / <kbd>Esc</kbd> on a `pick`      | Hands the chosen value back, or dismisses it ("Cancelled").            |
| `ext_check`, `ext_run`, `ext_test`, `ext_snap` | Work on an extension package; see [Extension tools](#extension-tools). |

Other keys work as everywhere in the panel; see [Keys](../usage.md#keys).

<details>
<summary>Each tool in full</summary>

| Tool                | What it does                                                                                                                  |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| `read_config`       | Returns both configuration files and the `lockedKeys` from a read-only `config.toml`. Works without the launcher.             |
| `validate_config`   | Checks a whole `config.toml` source as Dupin would load it, hotkey included, and writes nothing. Works without the launcher.  |
| `get_active_config` | Returns the values the launcher has applied, as `dupin config get` prints them.                                               |
| `open_panel`        | Shows the panel, optionally with a `query` typed in, as `dupin open --query` does.                                            |
| `pick`              | Shows `values` (1 to 1000 JSON values) in the panel and waits up to five minutes for you to choose one, as `dupin pick` does. |

`read_config` reports the base's `path`, `exists`, `readOnly` and TOML `source` at the top level. The `overlay`, what Settings saved over a read-only base, has `exists`, its TOML `source`, read from encrypted storage or from the running launcher that holds it, and `active`, which is true when the base is read-only. `lockedKeys` is empty for a writable base and null if the read-only base has invalid TOML syntax.

`validate_config` answers `{"valid": true, "config": {...}}` with the resulting values, or `{"valid": false, "problem": "line 2, column 1: ..."}`.

`pick` answers `{"value": ...}` with the chosen value unchanged.

</details>

## Notes

- `get_active_config`, `open_panel` and `pick` need the resident launcher running.
- No tool writes configuration, runs a script or opens a file ([#34](https://github.com/x-ryoppippi/dupin/issues/34)).

<details>
<summary>More notes</summary>

- The server starts no window. The agent runs elsewhere.
- To try it by hand, run `dupin mcp` and write JSON-RPC to its stdin.
- Writing configuration, running a script or opening a file have effects outside Dupin, and will ask for confirmation in the panel before they run.
- `open_panel` activates nothing, and <kbd>↩</kbd> does nothing for half a second after the panel appears.
- Each agent session starts its own `dupin mcp` process.

</details>

## Configuration

Add Dupin to a project's `.mcp.json`:

```json
{
	"mcpServers": {
		"dupin": { "command": "/Applications/Dupin.app/Contents/MacOS/dupin", "args": ["mcp"] }
	}
}
```

| Argument          | Default | Meaning                                                                        |
| ----------------- | ------- | ------------------------------------------------------------------------------ |
| `--workspace DIR` | None    | Also offers the [extension tools](#extension-tools) for packages inside `DIR`. |

<details>
<summary>Nix, Claude Code and how it works</summary>

With the Nix package or the home-manager module, `dupin` is on `PATH`, so `"command": "dupin"` works too. To add it for every project in Claude Code instead, run:

```sh
claude mcp add --scope user dupin -- dupin mcp
```

The server is a thin client of the same local control socket as the [`dupin` commands](./cli.md). It starts no window and holds no state of its own, and each agent session starts its own `dupin mcp` process.

</details>

## Extension tools

Started with `--workspace DIR`, the server also offers `ext_check`, `ext_run`, `ext_test` and `ext_snap` for packages inside that folder, so an agent can write a package and check it without the launcher. See [Testing and snapshots](./extension-testing.md#testing-and-snapshots).

<details>
<summary>The tools, their arguments and an example</summary>

```json
{
	"mcpServers": {
		"dupin": {
			"command": "dupin",
			"args": ["mcp", "--workspace", "extensions-src"]
		}
	}
}
```

| Tool        | Arguments                    | What it does                                                                                                                                                                                  |
| ----------- | ---------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ext_check` | `package`                    | Loads and compiles the package as `dupin ext check` does and returns its `id`, `name`, `version` and `keyword`.                                                                               |
| `ext_run`   | `package`, `query`           | Asks it one query as `dupin ext run --json` does. Its host requests fail as `unavailable`.                                                                                                    |
| `ext_test`  | `package`                    | Replays its fixtures as `dupin ext test --json` does and returns the report. It never rewrites them.                                                                                          |
| `ext_snap`  | `package`, `fixture`, `step` | Renders the panel after a fixture's step as `dupin ext snap` does and returns the PNG as image content, with its `step`, `width`, `height` and `scale`. `fixture` is relative to the package. |

`package` is a folder relative to the workspace. Without `--workspace`, these tools aren't listed and calling one is an unknown tool.

</details>

<details>
<summary>Security details</summary>

A `package` path that leads outside the workspace, by `..`, an absolute path or a symbolic link, is refused. The tools take prebuilt packages: no build, package manager or script runs, and nothing is written to the workspace. Guest code runs only in the sandboxed extension host. Host requests are answered from the fixture or not at all: the grants you made in Dupin don't apply, and nothing asks macOS for access. A failure is a tool error whose text is the diagnostic as JSON, with its `code`, `file`, `field`, position and `hint`.

</details>

## Errors

A tool that fails returns a result with `isError: true` and the reason as text, so the agent can correct itself; protocol mistakes are JSON-RPC errors.

<details>
<summary>Tool failures and JSON-RPC codes</summary>

| Failure                                | Reason text            |
| -------------------------------------- | ---------------------- |
| `open_panel` while Dupin isn't running | "Dupin is not running" |
| A `pick` the user dismisses            | "Cancelled"            |
| An argument it doesn't accept          | What is wrong          |

| Code     | For                                                    |
| -------- | ------------------------------------------------------ |
| `-32700` | Invalid JSON                                           |
| `-32600` | A malformed request, a batch or a message over 512 KiB |
| `-32601` | An unknown method                                      |
| `-32602` | An unknown tool or malformed `tools/call` parameters   |

</details>

<details>
<summary>Concurrency and cancelling</summary>

Tool calls run concurrently, so a `pick` waiting on the user does not hold up other requests. Cancelling a call (`notifications/cancelled`) closes its picker and the call gets no response. When the client closes stdin, any picker still waiting closes too.

</details>

## Protocol

Messages are JSON-RPC 2.0, one per line, on stdin and stdout; nothing else is written to stdout.

<details>
<summary>Methods and versions</summary>

The server implements `initialize`, `ping`, `tools/list` and `tools/call`, and accepts `notifications/initialized` and `notifications/cancelled`. It answers `initialize` with the client's protocol version when it is one of 2025-11-25, 2025-06-18, 2025-03-26 or 2024-11-05, and with 2025-11-25 otherwise.

</details>
