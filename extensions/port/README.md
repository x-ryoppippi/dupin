# Ports

Lists the ports your processes listen on, opens a web server's port in the browser, and stops the process holding one.

| Keyword       | `port`                                  |
| ------------- | --------------------------------------- |
| Command       | `port`                                  |
| On by default | No: turn it on in Settings → Extensions |
| User docs     | [Ports](../../docs/features/ports.md)   |

## How it's built

The smallest bundled example of `exec`.

- **`items()`**: runs `lsof` once, parses its `-F` output into one row per port and process, and answers with a `ttl` of 2 seconds, so Dupin refreshes it in the background while the panel shows it and typing never waits.
- **Actions:** **Open in Browser** (TCP only), **Copy Address** and **Copy Port** are plain actions. **Show Process** returns `enter` with the `proc` command and the PID, which only a bundled package may do for another package's command. **Terminate** asks to `confirm`, checks with `identify` that the PID still runs the program that listened, signals it, and calls `items.invalidate()`.
- **Capabilities:** `exec`, each run without a shell:

  | Profile     | Runs                                                        | Why                                                                                       |
  | ----------- | ----------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
  | `sockets`   | `/usr/sbin/lsof -nP +c 0 -iTCP -sTCP:LISTEN -iUDP -F cnPpu` | The listening sockets. `background`, so `items()` may run it, with up to 1 MiB of output. |
  | `identify`  | `/bin/ps -o comm= -p {pid}`                                 | What a PID runs now, before a signal is sent to it.                                       |
  | `terminate` | `/bin/kill -TERM {pid}`                                     | **Terminate**.                                                                            |

## Files

| Path              | What                                                                                     |
| ----------------- | ---------------------------------------------------------------------------------------- |
| `package.json`    | The manifest and its exec profiles.                                                      |
| `main.ts`         | The `lsof` parser, `items` and the actions.                                              |
| `fixtures/*.json` | The list, nothing listening, opening, copying, the hand-off to Processes, and Terminate. |

## Test it

```sh
dupin ext test extensions/port
dupin ext render extensions/port extensions/port/fixtures/list.json list.png
```

Each fixture answers the profiles from `replies.exec` with recorded `lsof` and `ps` output, so nothing runs; `empty.json` has `lsof` exit 1, as it does with no socket. Run `dupin` from this checkout's build, so the hand-off replays as a bundled package's; see [Working on them](../README.md#working-on-them).
