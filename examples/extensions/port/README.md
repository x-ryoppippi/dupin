# Ports

Lists the ports your processes listen on, opens a web server's port in the browser, and stops the process holding one.

| Keyword   | `port`                                          |
| --------- | ----------------------------------------------- |
| Command   | `port`                                          |
| Installed | No: an example; see its user docs to install it |
| User docs | [Ports](../../../docs/features/ports.md)        |

## How it's built

The smallest example of `exec`.

- **`items()`**: runs `lsof` once, parses its `-F` output into one row per port and process, and answers with a `ttl` of 2 seconds, so Dupin refreshes it in the background while the panel shows it and typing never waits.
- **Actions:** **Open in Browser** (TCP only), **Copy Address** and **Copy Port** are plain actions. **Terminate** asks to `confirm`, checks with `identify` that the PID still runs the program that listened, signals it, and calls `items.invalidate()`.
- **Capabilities:** `exec`, each run without a shell:

  | Profile     | Runs                                                        | Why                                                                                       |
  | ----------- | ----------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
  | `sockets`   | `/usr/sbin/lsof -nP +c 0 -iTCP -sTCP:LISTEN -iUDP -F cnPpu` | The listening sockets. `background`, so `items()` may run it, with up to 1 MiB of output. |
  | `identify`  | `/bin/ps -o comm= -p {pid}`                                 | What a PID runs now, before a signal is sent to it.                                       |
  | `terminate` | `/bin/kill -TERM {pid}`                                     | **Terminate**.                                                                            |

## Files

| Path              | What                                                          |
| ----------------- | ------------------------------------------------------------- |
| `package.json`    | The manifest and its exec profiles.                           |
| `main.ts`         | The `lsof` parser, `items` and the actions.                   |
| `fixtures/*.json` | The list, nothing listening, opening, copying, and Terminate. |

## Test it

```sh
dupin ext test examples/extensions/port
dupin ext render examples/extensions/port examples/extensions/port/fixtures/list.json list.png
```

Each fixture answers the profiles from `replies.exec` with recorded `lsof` and `ps` output, so nothing runs; `empty.json` has `lsof` exit 1, as it does with no socket.
