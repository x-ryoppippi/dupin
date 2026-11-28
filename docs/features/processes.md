# Processes

An [example extension](./extensions.md#example-extensions) that lists the processes running on your Mac, keeps the list current while you look at it, and stops one when you ask. It doesn't ship with Dupin; install it from [`examples/extensions/proc`](../../examples/extensions/proc).

## Install it

```sh
git clone https://github.com/x-ryoppippi/dupin
dupin ext install ./dupin/examples/extensions/proc
```

Then turn on the extensions folder and **Processes** in Settings → Extensions, and type `proc` <kbd>Space</kbd> and a name or a PID, such as `proc node` or `proc 4242`.

## What it does

- One row per process: its name, PID, user and command line, with its share of the CPU and its memory, as `ps` reported it within the last 2 seconds. While the panel shows the list, Dupin asks again every 2 seconds, and typing never waits for that.
- <kbd>↩</kbd> opens a page with the whole command line and the working folder. <kbd>→</kbd> or <kbd>⌘K</kbd> lists **Copy PID**, **Reveal in Finder**, **Open Activity Monitor**, **Terminate** (`SIGTERM`) and **Force Quit** (`SIGKILL`). Both signals ask you to confirm first.
- Terminate and Force Quit show only on your own processes, never on launchd or Dupin. Before either sends a signal, the extension checks that the PID still runs the program the row showed.
- The panel asks you to allow each program (`ps`, `lsof`, `kill`, `open`) the first time the extension would run it. The [package's README](../../examples/extensions/proc/README.md) lists each profile and how the extension is built.

When it was bundled, Processes had a **Show Open Ports** action that opened Ports with the PID typed. Only a bundled package may enter another package's command, so the example leaves that action out.
