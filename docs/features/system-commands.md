# System commands

Search for **Sleep**, **Lock Screen**, **Sleep Displays** or **Toggle Dark Mode**
alongside apps. Sleep suspends the Mac. Sleep Displays turns off the displays while
the Mac stays awake. Lock Screen locks the current session. Toggle Dark Mode switches
the system appearance.

Each command's row shows an SF Symbol, drawn in the colour of its text in light and
dark appearances: a moon for Sleep, a padlock for Lock Screen, a display for Sleep
Displays, a half-filled circle for Toggle Dark Mode, a circular arrow for Restart, a
power symbol for Shut Down, an arrow leaving a door for Log Out and a bin for Empty
Trash. The **Dupin Settings** and **Snapshots** rows show a gear and a viewfinder.

Enable **System Command Automation** in Dupin Settings under **General** to show
**Restart**, **Shut Down**, **Log Out** and **Empty Trash** in search. These commands
open a confirmation with **Cancel** selected. Select the command and press Enter again
to confirm. The commands send Apple events to loginwindow or Finder. macOS can ask
for Automation access once you confirm a command. If you deny access, review
Dupin under **System Settings → Privacy & Security → Automation**. Failures appear
in the launcher.

## Dupin's own commands

Search for **Restart Dupin** to relaunch Dupin, as **Relaunch** in Dupin Settings under
**General** does: a downloaded update is installed first, and running scripts are
cancelled. It shows a circular arrow.

Search for **Update Dupin** to restart into a downloaded update. When none is
downloaded, Dupin checks for one now and says what it found in the launcher: that it is
up to date, that an update is downloading or ready, that this copy can't replace itself
and where to download the update, or why the check failed. It shows a downward arrow in
a circle. Neither command needs System Command Automation. **Update Dupin** appears
only in release builds that update themselves, and not when `DUPIN_DISABLE_UPDATES=1`
is set or in Nix and cargo builds. See [Updates](updates.md).

## Configuration

```toml
system_automation = false # show Restart, Shut Down, Log Out and Empty Trash, which send Apple events
```
