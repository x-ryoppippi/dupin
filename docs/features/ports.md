# Ports

Dupin lists the ports your processes listen on and the process holding each, opens a web server's port in the browser, and stops the process when you ask.

## Open it

Switch on **Ports** in Settings → Extensions, then type `port` <kbd>Space</kbd>, and a port number, a program's name or a PID, such as `port 3000` or `port node`.

## What you see

- One row per port and process: the port, then the program, its PID and the addresses it listens on, with TCP or UDP at the row's end.
- A port a process listens on over both IPv4 and IPv6 is one row with both addresses.
- The list as `lsof` reported it within the last 2 seconds: while the panel shows it, Dupin asks again every 2 seconds, and typing never waits for that.

## What you can do

<kbd>↩</kbd> opens a TCP port in the browser; <kbd>→</kbd> or <kbd>⌘K</kbd> lists the rest:

| Key / action        | What it does                                                                  |
| ------------------- | ----------------------------------------------------------------------------- |
| **Open in Browser** | Opens `http://localhost:<port>`, for TCP                                      |
| **Copy Address**    | Copies `localhost:<port>`                                                     |
| **Copy Port**       | Copies the port number                                                        |
| **Show Process**    | Opens [Processes](./processes.md) with the PID typed, which lists the process |
| **Terminate**       | Asks the process to quit (`SIGTERM`), after you confirm                       |

Other keys: [Keys](../usage.md#keys).

## Notes

- Off by default, since it runs `lsof` and `kill`.
- macOS shows `lsof` only your own processes' sockets, so the list never has another user's or the system's ports.
- Connected and unbound UDP sockets listen on nothing, so they are left out.
- Before Terminate signals, Dupin checks that the PID still runs the program that listened, and refuses launchd and Dupin itself.

## Configuration

Ports has no setting in `config.toml`. Turn it on or off in Settings → Extensions, or ahead of time in `extension-grants.toml` beside `config.toml` ([Declaring them with Nix](./extensions.md#declaring-them-with-nix)):

```toml
version = 1

[packages."bundled:port"]
enabled = true
```

<details>
<summary>Extension scopes</summary>

| Scope            | Runs                                                        | When                                                     |
| ---------------- | ----------------------------------------------------------- | -------------------------------------------------------- |
| `exec:sockets`   | `/usr/sbin/lsof -nP +c 0 -iTCP -sTCP:LISTEN -iUDP -F cnPpu` | When Dupin asks for the list, without an action of yours |
| `exec:identify`  | `/bin/ps -o comm= -p <PID>`                                 | Before Terminate                                         |
| `exec:terminate` | `/bin/kill -TERM <PID>`                                     | For Terminate                                            |

Each runs without a shell; only the PID is filled in, and it can't start with `-`.

</details>

<details>
<summary>How it works and changing it</summary>

The list is TypeScript in [`extensions/port`](../../extensions/port), whose `items()` runs `lsof` through a [catalogue exec profile](./extension-permissions.md#exec). To change it, run `dupin ext eject port` and edit the copy, which is then your own extension, as [Overriding a built-in](./extensions.md#overriding-a-built-in) describes.

</details>
