# Snapshots

Press ⌘⇧S in the launcher to save a finished calculation, search or extension view.
Open **Snapshots** from Settings, press ⌘⇧O in the launcher, or type `Snapshots`
in the launcher and choose the Snapshots row. Its separate,
resizable window searches titles and saved content. Click a title or use ↑/↓ to
read it. Delete opens a confirmation before removing the selected snapshot;
⌘W closes the window. Editing the search field never deletes saved content.

Type `snap`, then Space, to list saved snapshots in the launcher by title, with
when each was saved, its mode and its query, newest first; typing filters them by
title. Enter opens the selected snapshot in the Snapshots window, and ⌘Enter
resumes its search, as ⌘R does, when its command is still available. The list
uses the metadata the Snapshots window reads, and reads the saved snapshots only when the
keyword is entered before the window has; the main search never reads them. See
[Keywords](./keywords.md) for `snap` and the other keywords.

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/snapshots/list-dark.png" />
<img alt="Saved snapshots listed under a Snapshots chip, each with when it was saved, its mode and its query." src="../images/snapshots/list-light.png" width="680" />
</picture>

Snapshots show historical content, including the visible result list, without
running saved actions. **Resume search** (⌘R) explicitly runs the original mode
and query again when its command is still available. Removed extensions leave
their saved content readable. Extension-specific JSON state is reserved in the
format; current commands resume through their query.

Snapshots hold what you typed and the results shown, so they are kept only in
[encrypted storage](./storage.md), in the `snapshots` namespace, one sealed record
per snapshot, and stay on this Mac: they never sync. There is no automatic capture
or telemetry. Storage accepts up to 100 entries, 512 KiB per entry and 8 MiB in
total; a save that exceeds a limit reports an error and removes nothing. If the
storage key can't be read, or the snapshots were sealed with another key, they stay
on disk untouched, saving is refused, and the Snapshots window and every Settings
page say why; nothing is ever written in the clear instead. A record that was
tampered with is dropped.

Use `dupin snapshots list --json` for saved Snapshot metadata, `dupin snapshots open
[--id ID]` to show the separate window, and `dupin snapshots close` to hide it.
The list returns the ID, saved time, mode, title and query, and flags storage
problems or truncation. These commands leave the saved snapshots and usage-ranking
history intact. Opening dismisses the launcher and may retain the completed query
for in-memory recall. An active picker blocks opening.
