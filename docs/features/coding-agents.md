# Coding agents

Dupin lists the AI coding agents running on your Mac, such as Claude Code, Codex and Gemini CLI, with the project each works in, so you can see what is running where and go to it.

## Open it

Switch on **Coding Agents** in Settings → Extensions, then type `agent` <kbd>Space</kbd>, and an agent's or a project's name to find one, such as `agent claude` or `agent shop`.

## What you see

- One row per running agent, newest first: the agent and its project folder's name, then the folder and its PID, after how long ago it started.
- Its share of the CPU and the memory it holds at the row's end.
- The list as of the last 5 seconds: while the panel shows it, Dupin asks again every 5 seconds, and typing never waits for that.

## What you can do

<kbd>↩</kbd> reveals the project in Finder; <kbd>→</kbd> or <kbd>⌘K</kbd> lists the rest:

| Key / action                 | What it does                                                                  |
| ---------------------------- | ----------------------------------------------------------------------------- |
| **Reveal Project in Finder** | Shows the folder the agent works in, selected in Finder                       |
| **Open Project in Terminal** | Opens a Terminal window in that folder                                        |
| **Copy Path**                | Copies the folder's path                                                      |
| **Show Process**             | Opens [Processes](./processes.md) with the PID typed, which lists the process |
| **Terminate**                | Asks the agent to quit (`SIGTERM`), after you confirm                         |

Other keys: [Keys](../usage.md#keys).

## Notes

- Off by default, since it runs `ps`, `lsof` and `kill`. Once it is on, the panel asks you to allow each program the first time it would run it; the ones that list run in the background, while you look at the list, and the rest only for an action you chose.
- It finds `aider`, `amp`, `claude`, `codex`, `copilot`, `crush`, `cursor-agent`, `gemini`, `goose`, `kimi`, `opencode` and `qwen`, by their program's name or by the script `node`, `bun`, `deno` or `python` runs. A helper an agent starts under its own name isn't listed again.
- It is for seeing what runs, not managing it: Dupin never reads an agent's conversation or files, only what `ps` and `lsof` report.

## Configuration

Coding Agents has no setting in `config.toml`, and preferences have no text field yet, so the names it looks for are in its package: to add one, eject it and add the name to `AGENTS` in `agents.ts`:

```sh
dupin ext eject agent
```

Turn it on or off in Settings → Extensions, or ahead of time in `extension-grants.toml` beside `config.toml` ([Declaring them with Nix](./extensions.md#declaring-them-with-nix)):

```toml
version = 1

[packages."bundled:agent"]
enabled = true
```

<details>
<summary>Extension scopes</summary>

| Scope            | Runs                                                             | When                                                     |
| ---------------- | ---------------------------------------------------------------- | -------------------------------------------------------- |
| `exec:processes` | `/bin/ps -axww -o pid=,ppid=,uid=,user=,%cpu=,rss=,etime=,comm=` | When Dupin asks for the list, without an action of yours |
| `exec:commands`  | `/bin/ps -ww -o pid=,args= -p <PIDs>`, for `node` and the like   | When Dupin asks for the list                             |
| `exec:cwd`       | `/usr/sbin/lsof -a -d cwd -F n -p <PIDs>`, for the agents found  | When Dupin asks for the list                             |
| `exec:identify`  | `/bin/ps -o comm= -p <PID>`                                      | Before Terminate                                         |
| `exec:terminate` | `/bin/kill -TERM <PID>`                                          | For Terminate                                            |
| `exec:terminal`  | `/usr/bin/open -a Terminal <folder>/`                            | For Open Project in Terminal                             |

Each runs without a shell. The folder Terminal opens ends in `/`, so a file in its place fails to open rather than running in Terminal.

</details>

<details>
<summary>How it works and changing it</summary>

The list is TypeScript in [`extensions/agent`](../../extensions/agent), whose `items()` runs `ps` and `lsof` through [`background` exec profiles](./extension-permissions.md#exec). A changed copy is your own extension, without the bundled one's grants, as [Overriding a built-in](./extensions.md#overriding-a-built-in) describes.

</details>
