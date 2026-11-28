# Ports

An [example extension](./extensions.md#example-extensions) that lists the ports your processes listen on and the process holding each, opens a web server's port in the browser, and stops the process when you ask. It doesn't ship with Dupin; install it from [`examples/extensions/port`](../../examples/extensions/port).

## Install it

```sh
git clone https://github.com/x-ryoppippi/dupin
dupin ext install ./dupin/examples/extensions/port
```

Then turn on the extensions folder and **Ports** in Settings → Extensions, and type `port` <kbd>Space</kbd> and a port number, a program's name or a PID, such as `port 3000` or `port node`.

## What it does

- One row per port and process: the port, the program, its PID and the addresses it listens on, as `lsof` reported them within the last 2 seconds. macOS shows `lsof` only your own processes' sockets.
- <kbd>↩</kbd> opens a TCP port at `http://localhost:<port>`. <kbd>→</kbd> or <kbd>⌘K</kbd> lists **Copy Address**, **Copy Port** and **Terminate**, which asks you to confirm. Before it sends `SIGTERM`, it checks that the PID still runs the program that listened, and it never signals launchd or Dupin.
- The panel asks you to allow `lsof`, `ps` and `kill` the first time the extension would run each. The [package's README](../../examples/extensions/port/README.md) lists each profile and how the extension is built.

When it was bundled, Ports had a **Show Process** action that opened Processes with the PID typed. Only a bundled package may enter another package's command, so the example leaves that action out.
