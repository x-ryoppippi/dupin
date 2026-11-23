# Window management

Turn on **Window Management** in Settings → Search, then type `window` or a
command name such as `Left Half`, `Centre Third` or `Maximise`. Enter hides
Dupin and arranges the focused window of the app that was frontmost when you
opened the panel.

Halves, thirds and Maximise use the display’s usable area, outside the Dock
and menu bar. Centre keeps the current size where it fits. Next Display
cycles through displays, keeping the window’s relative placement and shrinking
it only if it is too large for the destination.

Dupin requests Accessibility access when you enable Window Management in
Settings, or on the first command that needs it if you enabled it in
`config.toml`.
If access is missing, choose **Open Accessibility Settings** in the panel,
allow Dupin and run the command again.

## Configuration

```toml
window_management = false # show commands to move and resize windows in normal search
```
