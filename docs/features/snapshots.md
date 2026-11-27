# Snapshots

Snapshots save a finished calculation, search or extension view so you can read it again later.

## Open it

- <kbd>⇧⌘S</kbd> in the launcher: save the current view.
- <kbd>⇧⌘O</kbd> in the launcher, **Snapshots** in Settings, or type `Snapshots` and choose its row: open the Snapshots window.
- `snap` <kbd>Space</kbd>: list saved snapshots in the launcher.

## What you see

- In the launcher, saved snapshots by title, with when each was saved, its mode and its query, newest first.
- In the separate, resizable Snapshots window, a search over titles and saved content, and the selected snapshot's historical content, including the visible result list.

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/snapshots/list-dark.png" />
<img alt="Saved snapshots listed under a Snapshots chip, each with when it was saved, its mode and its query." src="../images/snapshots/list-light.png" width="680" />
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/snapshots/clip-dark.webp" />
<img alt="Typing snap and Space lists saved snapshots; ↓ selects one and → shows its actions." src="../images/snapshots/clip-light.webp" width="680" />
</picture>

## What you can do

In the `snap` list:

| Key / action  | What it does                                                                  |
| ------------- | ----------------------------------------------------------------------------- |
| Type          | Filter by title                                                               |
| <kbd>↩</kbd>  | Open the selected snapshot in the Snapshots window                            |
| <kbd>⌘↩</kbd> | Resume its search, as <kbd>⌘R</kbd> does, when its command is still available |

In the Snapshots window:

| Key / action                             | What it does                                                               |
| ---------------------------------------- | -------------------------------------------------------------------------- |
| Click a title, <kbd>↑</kbd> <kbd>↓</kbd> | Read it                                                                    |
| <kbd>⌘F</kbd>                            | Focus the search field                                                     |
| <kbd>⌘R</kbd> (**Resume search**)        | Run the original mode and query again, when its command is still available |
| **Delete**                               | Open a confirmation before removing the selected snapshot                  |
| <kbd>Esc</kbd>                           | Cancel a pending delete, else clear the search, else close the window      |
| <kbd>⌘W</kbd>                            | Close the window                                                           |

Other keys: [Keys](../usage.md#keys).

## Notes

- Nothing is captured automatically, and there is no telemetry.
- Snapshots hold what you typed and the results shown, so they are kept only in [encrypted storage](./storage.md) and stay on this Mac: they never sync.
- Storage accepts up to 100 entries, 512 KiB per entry and 8 MiB in total; a save that exceeds a limit reports an error and removes nothing.
- Viewing a snapshot never runs its saved actions. Editing the search field never deletes saved content.

<details>
<summary>Command line</summary>

```sh
dupin snapshots list --json    # saved snapshot metadata
dupin snapshots open [--id ID] # show the Snapshots window
dupin snapshots close          # hide it
```

The list returns the ID, saved time, mode, title and query, and flags storage problems or truncation. These commands leave the saved snapshots and usage-ranking history intact. Opening dismisses the launcher and may retain the completed query for in-memory recall. An active picker blocks opening.

The [MCP server](./mcp.md) offers read-only `list_snapshots`, `read_snapshot` and `open_snapshot` tools over the same control socket, so an agent you connected can read what you saved.

</details>

<details>
<summary>How the list and resuming work</summary>

The `snap` list uses the metadata the Snapshots window reads, and reads the saved snapshots only when the keyword is entered before the window has; the main search never reads them. See [Keywords](./keywords.md) for `snap` and the other keywords.

Removed extensions leave their saved content readable. Extension-specific JSON state is reserved in the format; current commands resume through their query.

</details>

<details>
<summary>Storage details</summary>

Snapshots live in the `snapshots` namespace, one sealed record per snapshot. If the storage key can't be read, or the snapshots were sealed with another key, they stay on disk untouched, saving is refused, and the Snapshots window and every Settings page say why; nothing is ever written in the clear instead. A record that was tampered with is dropped.

</details>
