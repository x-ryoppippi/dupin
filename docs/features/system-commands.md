# System commands

Sleep, lock or restart the Mac and more from the panel.

## Open it

Type a command's name, such as `sleep` or `lock`, in the main search.

## What you see

- One row per matching command, with an SF Symbol drawn in the colour of its text in light and dark appearances: a moon for Sleep, a padlock for Lock Screen, a display for Sleep Displays, a half-filled circle for Toggle Dark Mode, a circular arrow for Restart, a power symbol for Shut Down, an arrow leaving a door for Log Out and a bin for Empty Trash.
- The **Dupin Settings** and **Snapshots** rows show a gear and a viewfinder; **Restart Dupin** a circular arrow and **Update Dupin** a downward arrow in a circle.

## What you can do

| Key / action                                                               | What it does                                                       |
| -------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| <kbd>↩</kbd> on **Sleep**                                                  | Suspends the Mac                                                   |
| <kbd>↩</kbd> on **Sleep Displays**                                         | Turns off the displays while the Mac stays awake                   |
| <kbd>↩</kbd> on **Lock Screen**                                            | Locks the current session                                          |
| <kbd>↩</kbd> on **Toggle Dark Mode**                                       | Switches the system appearance                                     |
| <kbd>↩</kbd> on **Restart**, **Shut Down**, **Log Out** or **Empty Trash** | Opens a confirmation with **Cancel** selected                      |
| Select the command in the confirmation, then <kbd>↩</kbd>                  | Runs it                                                            |
| <kbd>↩</kbd> on **Restart Dupin**                                          | Relaunches Dupin, as **Relaunch** in Dupin Settings → General does |
| <kbd>↩</kbd> on **Update Dupin**                                           | Restarts into a downloaded update, or checks for one now           |

## Notes

- Restart, Shut Down, Log Out and Empty Trash are off by default; they show only with **System Command Automation** on.
- They send Apple events to loginwindow or Finder. macOS can ask for Automation access once you confirm a command. If you deny access, review Dupin under **System Settings → Privacy & Security → Automation**.
- Failures appear in the launcher.
- A [hotkey](./hotkeys.md#rows) runs a command without showing the panel; record one in Settings → Keywords, or bind `system/sleep` and the like. Restart, Shut Down, Log Out and Empty Trash still ask first: their shortcut shows their row, and <kbd>↩</kbd> opens the confirmation.
- **Restart Dupin** and **Update Dupin** don't need System Command Automation. **Update Dupin** appears only in release builds that update themselves, and not when `DUPIN_DISABLE_UPDATES=1` is set or in Nix and cargo builds. See [Updates](./updates.md).

<details>
<summary>Dupin's own commands</summary>

**Restart Dupin** installs a downloaded update first, and cancels running scripts.

When no update is downloaded, **Update Dupin** checks for one now and says what it found in the launcher: that it is up to date, that an update is downloading or ready, that this copy can't replace itself and where to download the update, or why the check failed.

</details>

## Configuration

```toml
system_automation = false # show Restart, Shut Down, Log Out and Empty Trash, which send Apple events
```

| Key                 | Default | Meaning                                                                                              |
| ------------------- | ------- | ---------------------------------------------------------------------------------------------------- |
| `system_automation` | `false` | Show Restart, Shut Down, Log Out and Empty Trash. Settings → General → **System Command Automation** |
