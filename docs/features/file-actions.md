# File actions

The action screen opens, moves, renames, copies or shares the selected file, folder or app.

## Open it

<kbd>→</kbd> at the end of an unselected input, <kbd>⌘K</kbd> or <kbd>⌃K</kbd> on a result.

## What you see

- The result's actions in this order: Open, Browse (folders), Open with…, Reveal in Finder, Quick Look, Copy name, Rename…, Move to…, Copy to…, Duplicate, Compress, Move to Trash and Send to….
- App results offer Open, Reveal in Finder, Quick Look and Send to….
- File search, content search, [folder browsing](./folder-browsing.md), [recent items](./recent-items.md) and a renamed result all offer the same file actions in the same order.

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/file-actions/actions-dark.png" />
<img alt="The action screen for Budget 2026.numbers in ~/Documents: Open, Open with…, Reveal in Finder, Quick Look, Copy name, Rename…, Move to… and Copy to…." src="../images/file-actions/actions-light.png" width="680" />
</picture>

## What you can do

| Key / action                   | What it does                                                                                                                                                                                 |
| ------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| <kbd>↑</kbd> <kbd>↓</kbd>      | Select an action                                                                                                                                                                             |
| <kbd>↩</kbd>                   | Run it                                                                                                                                                                                       |
| <kbd>←</kbd> or <kbd>Esc</kbd> | Return to the exact search and selection                                                                                                                                                     |
| **Open**                       | Opens the item                                                                                                                                                                               |
| **Browse**                     | Folders only: lists the folder in the panel ([Folder browsing](./folder-browsing.md))                                                                                                        |
| **Open with…**                 | Lists the apps registered for the item, its default handler first; <kbd>↩</kbd> opens it with the selected app, <kbd>←</kbd> or <kbd>Esc</kbd> returns to Actions without changing the query |
| **Reveal in Finder**           | Shows the item in Finder                                                                                                                                                                     |
| **Quick Look**                 | Opens the native preview; <kbd>Esc</kbd> returns to the picker with its search and selection intact                                                                                          |
| **Copy name**                  | Copies the item's name                                                                                                                                                                       |
| **Rename…**                    | Puts the name in the input, the part before its extension selected; <kbd>↩</kbd> renames it in the same folder, <kbd>Esc</kbd> returns to the search unchanged                               |
| **Move to…**, **Copy to…**     | Opens a Spotlight folder search; type a folder name, select it and <kbd>↩</kbd> to confirm, or <kbd>Esc</kbd> to return to the original search                                               |
| **Duplicate**                  | Copies the item beside itself: `report copy.pdf`, then `report copy 2.pdf`                                                                                                                   |
| **Compress**                   | Zips the item beside itself: `report.pdf.zip`, then `report.pdf 2.zip`                                                                                                                       |
| **Move to Trash**              | Opens a confirmation with Cancel selected; choose Move to Trash explicitly to continue                                                                                                       |
| **Send to…**                   | Shares or hands the item on ([Send to](./send-to.md))                                                                                                                                        |

## Notes

- Rename…, Move to…, Copy to…, Duplicate and Compress never replace an existing item.
- A held <kbd>↩</kbd> cannot confirm Move to Trash, and one that starts or confirms a rename does not repeat into the next step.
- Move to…, Copy to…, Duplicate and Compress run in the background and report their outcome in the panel.

<details>
<summary>Rename… rules</summary>

- A name that is empty, `.` or `..`, contains `/` or `:`, or is longer than 255 UTF-16 code units (as APFS counts) shows why and stays editable; <kbd>↩</kbd> does nothing until it is fixed.
- Renaming never replaces an existing item: if the name is taken, the rename fails and says so.
- After success the result shows the new name in place.

</details>

<details>
<summary>Move to… and Copy to… rules</summary>

- Move keeps the item's name and refuses an existing name or the item's current folder.
- Copy uses the original name if available, then Finder-style names such as `report copy.pdf` and `report copy 2.pdf`.
- A folder cannot be moved or copied into itself or one of its descendants.
- When moving between volumes, Dupin saves the copy first. If the source still matches, Dupin puts the original in the Trash so it can be recovered. If the source changed or could not be verified, Dupin keeps it in place and reports the problem.

</details>

<details>
<summary>Duplicate, Compress and Move to Trash</summary>

Duplicate copies the item beside itself as Finder does; duplicating a copy continues its numbering. Compress zips it beside itself with `ditto`, as Finder's Compress does. Both keep the launcher open, work in the background and report the new name, or the failure, in the launcher. Neither replaces an existing item.

After Move to Trash succeeds, the file leaves the current result list. Failures appear in the launcher.

</details>

[Folder browsing](./folder-browsing.md) · [Send to](./send-to.md)
