# Files

Dupin searches files by name or by content through Spotlight.

## Open it

- <kbd>Space</kbd> on an empty query: search file names ("Files").
- `in` <kbd>Space</kbd>: search inside files ("In files").

## What you see

- A chip with the mode, and a chip for each valid [qualifier](#qualifiers).
- Results as Spotlight finds them, improving as more candidates arrive, up to `file_limit` (200 by default).
- File names rank by filename match quality, your usage and previous query choices, last-used and modified dates, and path depth; cache and build directories are demoted by the [file ranking rules](#file-ranking).

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/files/clip-dark.webp" />
<img alt="Space on an empty query enters Files with its hint; ⌫ goes back to apps, and in then Space enters the In files search." src="../images/files/clip-light.webp" width="680" />
</picture>

## What you can do

| Key / action                                 | What it does                            |
| -------------------------------------------- | --------------------------------------- |
| <kbd>↩</kbd>                                 | Open the file                           |
| <kbd>⌘↩</kbd>                                | Reveal in Finder                        |
| <kbd>⌘Y</kbd>                                | Quick Look                              |
| <kbd>⇥</kbd> on a folder                     | [Browse](./folder-browsing.md) it       |
| <kbd>→</kbd>, <kbd>⌘K</kbd> or <kbd>⌃K</kbd> | Show its [actions](./file-actions.md)   |
| <kbd>⌫</kbd> on a qualifier chip             | Delete the whole chip; Undo restores it |
| <kbd>⌫</kbd> on an empty query               | Return to app search                    |

Other keys: [Keys](../usage.md#keys).

## Notes

- Both modes find only what Spotlight has indexed.
- Files: every word of the query must appear in the file name or display name.
- In files: every word must start a word in the text Spotlight indexed from inside the file.
- You can paste a complete query such as `in release notes` or `d serendipity` into app search. Dupin accepts the same keywords and configured aliases, switches mode, and leaves the search term in the input.
- A selected file or an open action screen keeps its target while results reorder.

## Qualifiers

Narrow either mode with qualifiers: `report kind:pdf modified:7d`.

| Qualifier        | Meaning                                                                     |
| ---------------- | --------------------------------------------------------------------------- |
| `kind:pdf`       | A category: pdf, image, text, audio, video, folder, archive or app          |
| `ext:rs`         | An extension, including compound extensions such as `ext:tar.gz`            |
| `in:~/Documents` | Search within an absolute or home-relative folder                           |
| `modified:7d`    | Modified within a positive number of hours (`h`), days (`d`) or weeks (`w`) |

<details>
<summary>Qualifier rules</summary>

- Combine different qualifiers, using each at most once.
- Valid qualifiers appear as inline chips without changing the query text.
- Quote folder paths containing spaces, such as `in:"~/My Documents"`. Backslashes can escape spaces, quotes and backslashes in qualifier values.
- Unknown colon prefixes remain ordinary search text.
- Folder scopes resolve symlinks and require an existing directory. A missing path or a path to a file returns no results.
- Incomplete or invalid qualifiers show a syntax hint and return no results.
- File-name search accepts qualifiers without ordinary terms; content search still requires a word to search for.
- Copy and query recall retain the raw query.

</details>

## Configuration

### File ranking

```toml
file_limit = 200 # the most files a Spotlight search shows
file_demotions = ["~/Library", "Caches", "node_modules", ".git", "target", "build", "dist"]
```

| Key              | Default  | Meaning                                                                                    |
| ---------------- | -------- | ------------------------------------------------------------------------------------------ |
| `file_limit`     | `200`    | The most results shown, from a bounded candidate pool; `0` displays none. Capped at 10,000 |
| `file_demotions` | as above | Directories ranked lower without excluding them; `[]` turns this off                       |

<details>
<summary>Ranking details</summary>

File-name search gathers up to four times `file_limit` candidates, ranks them, then applies the display limit. Both limits are capped at 10,000. Better candidates can replace early results while the search runs. Selecting a row or opening its actions keeps that file available even if it falls below the normal display cut.

In `file_demotions`, absolute paths and `~/` paths match directory prefixes; relative entries match directory-component sequences, ignoring case. Entries do not match file names. Edits reload automatically and refresh an active file-name search. Content search keeps Spotlight's order and does not use these rules.

</details>

[Folder browsing](./folder-browsing.md) · [File actions](./file-actions.md)
