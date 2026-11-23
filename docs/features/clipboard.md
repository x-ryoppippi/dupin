# Clipboard history

Turn on clipboard history in Settings → Search (it is off by default; see [configuration](#configuration)). Then type `clip` and Space: a "Clipboard" chip shows the mode and the list starts with your most recent copies, each with how long ago it was copied. Type to keep only entries containing every word, ignoring case. ↑ and ↓ move through the list even with an empty query. Clipboard search terms are not kept for query recall. Enter copies the selected entry back to the clipboard and hides the panel, ready to paste; file entries copy the files, as Finder's Copy does. Dupin does not paste into the frontmost app for you.

The action screen (→, ⌘K or Ctrl+K) offers Copy, Delete from History and Clear History. Deleting removes that entry only; the clipboard itself keeps whatever it holds. Clear History asks for confirmation with Cancel selected before forgetting every entry. Clipboard results can't be saved as snapshots and are not recorded in usage-ranking history.

## Configuration

Clipboard history is off by default because it records what you copy. Turn it on with the switch in Settings → Search, or in `config.toml`:

```toml
[clipboard]
enabled = false # keep text and files copied while Dupin runs, searchable with `clip`
max_entries = 200 # 1 to 1000; the oldest entry goes first
max_entry_bytes = 65536 # 1024 to 1048576; larger copies are skipped, never truncated
ignored_apps = [
	"com.apple.keychainaccess",
	"com.apple.Passwords",
	"com.1password.1password",
	"com.agilebits.onepassword7",
	"com.bitwarden.desktop",
]
```

While it is on, Dupin checks the clipboard's change count twice a second on a background thread and records each new copy: plain text, or the files copied in Finder. Images and rich text are not recorded yet. Copies already on the clipboard when the history is turned on are not recorded. Copying something already in the history moves it to the top.

Copies that password managers and other apps mark as concealed, transient or auto-generated ([nspasteboard.org](http://nspasteboard.org) types) are never read. Nor are copies that name an app in `ignored_apps` as their source, or that land while such an app is frontmost: Dupin checks the apps frontmost at the polls just before and just after a copy, so switching apps several times within half a second can slip past it. Rely on the concealed marker, which password managers set, for secrets. Setting `ignored_apps` replaces the defaults; find an app's identifier with `osascript -e 'id of app "Name"'`.

The history lives only in Dupin's memory. It is never written to disk, included in snapshots or logged, and it is gone when Dupin quits. Turning it off stops watching and forgets every entry at once. On macOS versions that report the pasteboard access policy, background recording requires **Always Allow**. Under the default policy, `clip` offers a one-time clipboard read so macOS can prompt and list Dupin in System Settings. If macOS is set to ask or deny, the row opens Privacy & Security, where you can always allow Dupin to paste from other apps.
