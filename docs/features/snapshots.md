# Snapshots

Press ⌘⇧S in the launcher to save a finished calculation, search or extension view.
Open **Snapshots** from Settings, or press ⌘⇧O in the launcher. Its separate,
resizable window searches titles and saved content. Click a title or use ↑/↓ to
read it. Delete opens a confirmation before removing the selected snapshot;
⌘W closes the window. Editing the search field never deletes saved content.

Snapshots show historical content, including the visible result list, without
running saved actions. **Resume search** (⌘R) explicitly runs the original mode
and query again when its command is still available. Removed extensions leave
their saved content readable. Extension-specific JSON state is reserved in the
format; current commands resume through their query.

Snapshots stay locally in Dupin's application-support directory (or
`$XDG_DATA_HOME/dupin/snapshots` when set), with private permissions. There is no
automatic capture or telemetry. Storage accepts up to 100 entries, 512 KiB per
entry and 8 MiB in total; a save that exceeds a limit reports an error. Invalid
or unsupported archives remain on disk and block edits, with the problem shown
in the Snapshots window.

Use `dupin snapshots list --json` for saved Snapshot metadata, `dupin snapshots open
[--id ID]` to show the separate window, and `dupin snapshots close` to hide it.
The list returns the ID, saved time, mode, title and query, and flags archive
problems or truncation. These commands leave the saved archive and usage-ranking
history intact. Opening dismisses the launcher and may retain the completed query
for in-memory recall. An active picker blocks opening.
