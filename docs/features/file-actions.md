# File actions

Right Arrow at the end of an unselected input, or Cmd+K, opens the selected
result's action screen: Open, Browse (folders), Open with…, Reveal in Finder, Quick Look, Copy name, Rename…, Move to…, Copy to…, Duplicate, Compress, Move to Trash and Send to…. Escape cancels the
screen and returns to the exact search and selection. Use ↑ and ↓ to select an action, Enter to run it, and Left or Escape to return. Ctrl+K also opens actions. App results offer Open, Reveal, Quick Look and Send to…. File search, content search, [folder browsing](./folder-browsing.md), [recent items](./recent-items.md) and a renamed result all offer the same file actions in the same order.

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/file-actions/actions-dark.png" />
<img alt="The action screen for Budget 2026.numbers in ~/Documents: Open, Open with…, Reveal in Finder, Quick Look, Copy name, Rename…, Move to… and Copy to…." src="../images/file-actions/actions-light.png" width="680" />
</picture>

Open with… lists the applications registered for the selected file or folder, with its default handler first. Choose an application to open the item. Left or Escape returns to Actions without changing the query. Move to Trash opens a confirmation with Cancel selected; choose Move to Trash explicitly to continue. Held Enter cannot confirm. After success the file leaves the current result list. Failures appear in the launcher.

Rename… puts the item's name in the input, with the part before its extension selected, as Finder does. Type the new name and press Enter to rename it in the same folder, or Escape to return to the search unchanged. A name that is empty, `.` or `..`, contains `/` or `:`, or is longer than 255 UTF-16 code units (as APFS counts) shows why and stays editable; Enter does nothing until it is fixed. A held Enter that starts or confirms a rename does not repeat into the next step. Renaming never replaces an existing item: if the name is taken, the rename fails and says so. After success the result shows the new name in place.

Move to… and Copy to… open a Spotlight folder search in the panel. Type a folder name, select a result and press Enter to confirm, or press Escape to return to the original search. The operation runs in the background and reports its outcome in the panel. Move keeps the item's name and refuses an existing name or the item's current folder. Copy uses the original name if available, then Finder-style names such as `report copy.pdf` and `report copy 2.pdf`. A folder cannot be moved or copied into itself or one of its descendants.

When moving between volumes, Dupin saves the copy first. If the source still matches, Dupin puts the original in the Trash so it can be recovered. If the source changed or could not be verified, Dupin keeps it in place and reports the problem.

Duplicate copies the item beside itself as Finder does: `report copy.pdf`, then `report copy 2.pdf`, and so on; duplicating a copy continues its numbering. Compress zips it beside itself with `ditto`, as Finder's Compress does: `report.pdf.zip`, then `report.pdf 2.zip`. Both keep the launcher open, work in the background and report the new name, or the failure, in the launcher. Neither replaces an existing item.

Quick Look opens the native preview; Escape returns to the picker with its search and selection intact.

[Folder browsing](./folder-browsing.md) · [Send to](./send-to.md)
