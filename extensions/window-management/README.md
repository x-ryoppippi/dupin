# Window Management

Moves and resizes the focused window to a half, a third, the whole screen or the next display.

| Keyword       | `window`, or a command's name in the main search              |
| ------------- | ------------------------------------------------------------- |
| Command       | `window-management`                                           |
| On by default | No: turn it on in Settings → Extensions                       |
| User docs     | [Window management](../../docs/features/window-management.md) |

A port of the former native window management, now an optional extension; see [Built-ins as bundled extensions](../../docs/design/extension-platform.md#built-ins-as-bundled-extensions).

## How it's built

- **`items()`**: ten fixed commands, each with other spellings (`Center`, `Maximize`) and `window <name>` as `keywords`, and one `run` action.
- **`defaultSearch: true`**: the commands are listed in the main search too, so `left half` finds one without the keyword.
- **`actions.arrange(data)`**: reads the displays and the focused window, works the target frame out in the guest, moves the window and returns `hide`, or a `toast` when there is no window or the move fails. On `needs-consent` it returns nothing, since Dupin shows the request to allow above the results.
- **Capabilities:** `windows: ["read", "arrange"]`, for `windows.displays()`, `windows.focused()` and `windows.setFrame(frame)`. Both need macOS Accessibility access, which turning the package on asks for.

## Files

| Path              | What                                                                                              |
| ----------------- | ------------------------------------------------------------------------------------------------- |
| `package.json`    | The manifest.                                                                                     |
| `main.ts`         | The commands, the frame each one targets, and `arrange`.                                          |
| `fixtures/*.json` | Listing the commands, consent, no focused window, one display, two displays, a window across two. |

## Test it

```sh
dupin ext test extensions/window-management
dupin ext render extensions/window-management extensions/window-management/fixtures/browse.json browse.png
```

Each fixture gives the displays and the focused window in `replies.windows`. A move goes through the same bounds as on a Mac and updates the focused window, so each step's `windows` outcome shows where it went and the next step starts there. Run `dupin` from this checkout's build; see [Working on them](../README.md#working-on-them).
