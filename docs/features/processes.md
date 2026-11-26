# Processes

Dupin lists the processes running on your Mac, keeps the list current while you look at it, and stops one when you ask.

## Open it

Switch on **Processes** in Settings → Extensions, then type `proc` <kbd>Space</kbd>, and a name or a PID to find one, such as `proc node` or `proc 4242`.

## What you see

- One row per process: its name, then its PID, its user and its command line, cut to fit, after how long ago it started.
- Its share of the CPU and the memory it holds at the row's end.
- The list as `ps` reported it within the last 2 seconds: while the panel shows it, Dupin asks again every 2 seconds and updates the rows, and typing never waits for that.

## What you can do

<kbd>↩</kbd> opens a process's details; <kbd>→</kbd> or <kbd>⌘K</kbd> lists the rest:

| Key / action              | What it does                                                                                 |
| ------------------------- | -------------------------------------------------------------------------------------------- |
| **Details**               | Shows the whole command line and the working folder on a page of its own, with these actions |
| **Copy PID**              | Copies the process ID                                                                        |
| **Reveal in Finder**      | Shows the program the process runs, selected in Finder                                       |
| **Open Activity Monitor** | Opens Activity Monitor                                                                       |
| **Show Open Ports**       | Opens [Ports](./ports.md) with the PID typed, which lists what the process listens on        |
| **Terminate**             | Asks the process to quit (`SIGTERM`), after you confirm                                      |
| **Force Quit**            | Stops it at once (`SIGKILL`), after you confirm; it can't save anything first                |

Other keys: [Keys](../usage.md#keys).

## Notes

- Off by default, since it runs `ps`, `lsof` and `kill`.
- Terminate and Force Quit show only on your own processes, never on launchd, on another user's process or on Dupin itself. Before sending a signal, Dupin checks that the PID still runs the program the row showed, since a PID that ended can be reused, and says so rather than stopping another program.
- The command line in a row is cut short; Details reads the whole of it, and the working folder, when you open it.
- Dupin lists every process in parts of 400, by PID, so even a busy Mac's thousand or more arrive whole.

## Configuration

Processes has no setting in `config.toml`. Turn it on or off in Settings → Extensions, or ahead of time in `extension-grants.toml` beside `config.toml` ([Declaring them with Nix](./extensions.md#declaring-them-with-nix)):

```toml
version = 1

[packages."bundled:proc"]
enabled = true
```

<details>
<summary>Extension scopes</summary>

Processes names each program it runs, so you pick none, and holds these scopes while it runs as shipped:

| Scope                   | Runs                                                             | When                                                     |
| ----------------------- | ---------------------------------------------------------------- | -------------------------------------------------------- |
| `exec:processes`        | `/bin/ps -axww -o pid=,ppid=,uid=,user=,%cpu=,rss=,etime=,comm=` | When Dupin asks for the list, without an action of yours |
| `exec:commands`         | `/bin/ps -ww -o pid=,args= -p <PIDs>`                            | When Dupin asks for the list, and for Details            |
| `exec:cwd`              | `/usr/sbin/lsof -a -d cwd -F n -p <PID>`                         | For Details                                              |
| `exec:identify`         | `/bin/ps -o comm= -p <PID>`                                      | Before Terminate or Force Quit                           |
| `exec:terminate`        | `/bin/kill -TERM <PID>`                                          | For Terminate                                            |
| `exec:force-quit`       | `/bin/kill -KILL <PID>`                                          | For Force Quit                                           |
| `exec:activity-monitor` | `/usr/bin/open -b com.apple.ActivityMonitor`                     | For Open Activity Monitor                                |

Each runs without a shell, with only the PID or PIDs Dupin fills in, which can't start with `-`, so it can't become an option or a process group. Revoking a scope there stops what it runs.

</details>

<details>
<summary>How it works and changing it</summary>

The list is TypeScript in [`extensions/proc`](../../extensions/proc): its `items()` runs `ps` through two [catalogue exec profiles](./extension-permissions.md#exec), and Dupin matches the rows it holds as you type. Show Open Ports returns an [`enter` effect](./extension-api.md#effects-on-pages) for the Ports command.

To change it, eject it and edit the copy:

```sh
dupin ext eject proc
```

A changed copy is your own extension, without the bundled one's grants, as [Overriding a built-in](./extensions.md#overriding-a-built-in) describes.

</details>
