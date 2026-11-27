# Coding Agents

Lists the AI coding agents running on your Mac, such as Claude Code, Codex and Gemini CLI, with the project folder each works in.

| Keyword       | `agent`                                               |
| ------------- | ----------------------------------------------------- |
| Command       | `agent`                                               |
| On by default | No: turn it on in Settings → Extensions               |
| User docs     | [Coding agents](../../docs/features/coding-agents.md) |

## How it's built

- **`items()`**: runs `ps` for every process, reads the command lines of interpreters (`node`, `bun`, `deno`, `python`, `python3`) to find agents written as scripts, drops a helper an agent starts under its own name, reads each agent's working folder with `lsof`, and answers up to 200 rows, newest first, with a `ttl` of 5 seconds.
- **`agents.ts`** names the agents it looks for; add a name there, in an ejected copy, to list another.
- **Actions:** **Reveal Project in Finder** (<kbd>↩</kbd>) and **Copy Path** are plain actions. **Open Project in Terminal** runs its profile and hides Dupin. **Show Process** returns `enter` with the `proc` command and the PID, which only a bundled package may do for another package's command. **Terminate** asks to `confirm`, is offered only on your own processes, and signals only while `identify` shows the PID still runs the same program.
- **Capabilities:** `exec`, each run without a shell:

  | Profile     | Runs                                                             | Why                                                      |
  | ----------- | ---------------------------------------------------------------- | -------------------------------------------------------- |
  | `processes` | `/bin/ps -axww -o pid=,ppid=,uid=,user=,%cpu=,rss=,etime=,comm=` | Every process. `background`, with up to 1 MiB of output. |
  | `commands`  | `/bin/ps -ww -o pid=,args= -p {pids}`                            | Interpreters' command lines. `background`, up to 1 MiB.  |
  | `cwd`       | `/usr/sbin/lsof -a -d cwd -F n -p {pids}`                        | Each agent's project folder. `background`.               |
  | `identify`  | `/bin/ps -o comm= -p {pid}`                                      | What a PID runs now, before a signal is sent to it.      |
  | `terminate` | `/bin/kill -TERM {pid}`                                          | **Terminate**.                                           |
  | `terminal`  | `/usr/bin/open -a Terminal {folder}/`                            | **Open Project in Terminal**.                            |

## Files

| Path              | What                                                                                                                                                                                                                                       |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `package.json`    | The manifest and its exec profiles.                                                                                                                                                                                                        |
| `main.ts`         | Finding agents, `items` and the actions.                                                                                                                                                                                                   |
| `agents.ts`       | The agents' names and the interpreters that run scripts.                                                                                                                                                                                   |
| `ps.ts`           | Parsing `ps` and `lsof` output, and sending signals. [`proc/ps.ts`](../proc/ps.ts) is the same file, since a package can import only from its own folder; `cargo test -p dupin a_module_bundled_packages_share` fails once the two differ. |
| `fixtures/*.json` | The list, revealing, copying, opening a terminal, the hand-off to Processes, and Terminate.                                                                                                                                                |

## Test it

```sh
dupin ext test extensions/agent
dupin ext render extensions/agent extensions/agent/fixtures/list.json list.png
```

Each fixture answers the profiles from `replies.exec` with recorded `ps` and `lsof` output, so nothing runs. Run `dupin` from this checkout's build, so the hand-off replays as a bundled package's; see [Working on them](../README.md#working-on-them).
