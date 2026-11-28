# Window management

An [example extension](./extensions.md#example-extensions) that moves and resizes the front app's focused window to a half, a third, the whole screen or the next display. It doesn't ship with Dupin; install it from [`examples/extensions/window-management`](../../examples/extensions/window-management). Dupin keeps the `windows` capability it runs on, so any extension can arrange windows the same way.

## Install it

```sh
git clone https://github.com/x-ryoppippi/dupin
dupin ext install ./dupin/examples/extensions/window-management
```

Then turn on the extensions folder and **Window Management** in Settings → Extensions, and type a command name such as `Left Half`, `Centre Third` or `Maximise` in the main search, or `window` <kbd>Space</kbd> to list them all.

## What it does

- **Left Half**, **Right Half**, **Top Half**, **Bottom Half**, **Left Third**, **Centre Third**, **Right Third**, **Maximise**, **Centre** and **Next Display**, each over the display's usable area, outside the Dock and the menu bar. `Center`, `Center Third` and `Maximize` find the same commands.
- <kbd>↩</kbd> arranges the focused window of the app that was in front when you opened the panel, then hides Dupin. A window that spans displays is arranged on the one it overlaps most.
- It needs macOS Accessibility access and the `windows:read` and `windows:arrange` scopes. The first command you choose asks for whichever is missing. The [package's README](../../examples/extensions/window-management/README.md) has how the extension is built.

A [hotkey](./hotkeys.md#rows) arranges the front window without showing the panel. Name the command as `window-management/` followed by its ID: `left-half`, `right-half`, `top-half`, `bottom-half`, `left-third`, `centre-third`, `right-third`, `maximise`, `centre` or `next-display`.

```toml
[bindings."ctrl+alt+l"]
target = "dupin"
command = "window-management/left-half"
```

Dupin still accepts the `window_management` key that Settings once wrote to `config.toml`. It ignores the key and leaves it out the next time it saves.
