# Processes

Lists the processes running on your Mac, shows one's details, and stops one when you ask.

| Keyword   | `proc`                                           |
| --------- | ------------------------------------------------ |
| Command   | `proc`                                           |
| Installed | No: an example; see its user docs to install it  |
| User docs | [Processes](../../../docs/features/processes.md) |

## How it's built

- **`items(ctx)`**: runs `ps` for every process, then reads the command lines of one part at a time, sending the catalogue in parts by PID with `more` and a cursor, and a `ttl` of 2 seconds so Dupin refreshes it while the panel shows it.
- **Actions:** **Details** (<kbd>↩</kbd>) reads the whole command line and working folder and returns `push` of the `details` view. **Copy PID** and **Reveal in Finder** are plain actions. **Open Activity Monitor** runs its profile. **Terminate** and **Force Quit** ask to `confirm`, are offered only on your own processes, and signal only while `identify` shows the PID still runs the program the row showed.
- **`views.details`**: a `detail` page with the PID, user, program and folder as metadata, the command line as Markdown, and the process's actions with **Copy Command** added.
- **Capabilities:** `exec`, each run without a shell:

  | Profile            | Runs                                                             | Why                                                      |
  | ------------------ | ---------------------------------------------------------------- | -------------------------------------------------------- |
  | `processes`        | `/bin/ps -axww -o pid=,ppid=,uid=,user=,%cpu=,rss=,etime=,comm=` | Every process. `background`, with up to 1 MiB of output. |
  | `commands`         | `/bin/ps -ww -o pid=,args= -p {pids}`                            | Command lines. `background`, with up to 1 MiB of output. |
  | `cwd`              | `/usr/sbin/lsof -a -d cwd -F n -p {pids}`                        | The working folder, for **Details** only.                |
  | `identify`         | `/bin/ps -o comm= -p {pid}`                                      | What a PID runs now, before a signal is sent to it.      |
  | `terminate`        | `/bin/kill -TERM {pid}`                                          | **Terminate**.                                           |
  | `force-quit`       | `/bin/kill -KILL {pid}`                                          | **Force Quit**.                                          |
  | `activity-monitor` | `/usr/bin/open -b com.apple.ActivityMonitor`                     | **Open Activity Monitor**.                               |

## Files

| Path              | What                                                      |
| ----------------- | --------------------------------------------------------- |
| `package.json`    | The manifest and its exec profiles.                       |
| `main.ts`         | `items`, the `details` view and the actions.              |
| `ps.ts`           | Parsing `ps` and `lsof` output, and sending signals.      |
| `fixtures/*.json` | The list, consent, details, and Terminate and Force Quit. |

## Test it

```sh
dupin ext test examples/extensions/proc
dupin ext render examples/extensions/proc examples/extensions/proc/fixtures/list.json list.png
```

Each fixture answers the profiles from `replies.exec` with recorded `ps` and `lsof` output, so nothing runs; `consent.json` leaves listing processes `pending` with `replies.consent`.
