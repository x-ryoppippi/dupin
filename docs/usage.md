# Using Dupin

Dupin lives in the background with no Dock icon or menu bar. Its hotkey (⌘Space by default; see [configuration](./configuration.md)) shows the panel; pressing it again, Esc, or clicking elsewhere hides it.

## Keys

| Key                                     | In the panel                                          |
| --------------------------------------- | ----------------------------------------------------- |
| Hotkey                                  | Show or hide the panel                                |
| Type                                    | Fuzzy-search installed apps                           |
| Space on an empty query                 | Search files with Spotlight, by file and display name |
| `in` then Space                         | Search inside files with Spotlight, as Alfred's `in`  |
| `define` or `d`, then Space             | Look up a word in macOS dictionaries                  |
| `clip` then Space                       | Search the clipboard history, when it is on           |
| ⌫ on an empty query                     | Leave a folder or the current search mode             |
| ↑ ↓                                     | Move the selection                                    |
| Tab at the end of an unselected input   | Complete the selected keyword, or browse the folder   |
| ↑ on an empty query, then ↑ ↓           | Recall searches in the current mode                   |
| ↩                                       | Open the app or file                                  |
| ⌘↩                                      | Reveal the file in Finder                             |
| ⌘Y                                      | Preview the selected app or file with Quick Look      |
| → at the end of the input, ⌘K or Ctrl+K | Show actions for the selected result                  |
| Esc, or clicking elsewhere              | Hide the panel                                        |
| ⌘Q                                      | Quit Dupin                                            |
| ⌘,                                      | Open Settings                                         |

Completed searches are remembered for the current Dupin process when you clear
the input, hide the panel or activate a result. With an empty input, ↑ recalls the
latest term in the current mode from the first row; further ↑ and ↓ walk through those terms and
back to the empty input. Editing a recalled term restores normal result selection.
This history holds up to 100 terms and is separate from app-ranking history.

## Features

- [Apps](./features/apps.md)
- [Recent items](./features/recent-items.md)
- [Files](./features/files.md)
- [Folder browsing](./features/folder-browsing.md)
- [File actions](./features/file-actions.md)
- [Send to](./features/send-to.md)
- [Dictionary](./features/dictionary.md)
- [Web searches](./features/web-searches.md)
- [Maps](./features/maps.md)
- [Calculator](./features/calculator.md)
- [Calendar](./features/calendar.md)
- [Clipboard history](./features/clipboard.md)
- [Snippets](./features/snippets.md)
- [Script commands](./features/scripts.md)
- [Hotkeys](./features/hotkeys.md)
- [Search selected text](./features/selected-text.md)
- [Window management](./features/window-management.md)
- [System commands](./features/system-commands.md)
- [Snapshots](./features/snapshots.md)
- [Extensions](./features/extensions.md)
- [Settings](./features/settings.md)
- [Permissions](./features/permissions.md)
- [Updates](./features/updates.md)
- [Appearance](./features/appearance.md)
- [MCP server](./features/mcp.md)
