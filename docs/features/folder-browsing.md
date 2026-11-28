# Folder browsing

Dupin lists a folder's contents in the panel so you can walk through it.

## Open it

- <kbd>⇥</kbd> on a folder, or its **Browse** action.
- Type a path starting with `~/` or `/` in the main search: `~/Documents/rep` opens `~/Documents` and filters it by `rep`.

## What you see

- The folder's name in the chip.
- Its entries, folders first, then by name.

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/folder-browsing/browse-dark.png" />
<img alt="The Reports folder listed in the panel under a Reports chip, with a Filter folder field: Annual report 2026.pdf, Expense report.numbers, Quarterly report.pdf and report-notes.md." src="../images/folder-browsing/browse-light.png" width="680" />
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/folder-browsing/clip-dark.webp" />
<img alt="Tab on the Reports folder lists its files; typing q filters them, and ⌫ on an empty filter returns to the search." src="../images/folder-browsing/clip-light.webp" width="680" />
</picture>

## What you can do

| Key / action                    | What it does                                                                                                                                                                                              |
| ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Type                            | Filter the entries fuzzily                                                                                                                                                                                |
| Type `.` first                  | Show hidden entries too                                                                                                                                                                                   |
| <kbd>⇥</kbd>                    | Enter the selected subfolder                                                                                                                                                                              |
| Type a subfolder's name and `/` | Enter it; a name that is no folder stays in the filter                                                                                                                                                    |
| <kbd>⌫</kbd> on an empty filter | Return to the parent folder with its filter, or with the name a typed path left there; from the first folder, restore the search and its query, and the selection <kbd>⇥</kbd> or **Browse** started from |

Listed entries have the same [actions](./file-actions.md) as search results. Other keys: [Keys](../usage.md#keys).
