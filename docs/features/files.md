# Files

Space on an empty query switches to file search; a "Files" chip shows the mode. Every word of the query must appear in the file name or display name. Results appear as Spotlight finds them, then improve as more candidates arrive. Ranking combines filename match quality, your usage and previous query choices, last-used and modified dates, and path depth. Cache and build directories are demoted by the configurable [file ranking rules](#file-ranking).

The list shows up to `file_limit` results (200 by default) from a bounded candidate pool. A selected file or an open action screen keeps its target while results reorder.

`in` followed by Space switches to content search ("In files"). Every word must start a word in the text Spotlight indexed from inside the file.

Narrow either file mode with qualifiers:

| Qualifier        | Meaning                                                                     |
| ---------------- | --------------------------------------------------------------------------- |
| `kind:pdf`       | A category: pdf, image, text, audio, video, folder, archive or app          |
| `ext:rs`         | An extension, including compound extensions such as `ext:tar.gz`            |
| `in:~/Documents` | Search within an absolute or home-relative folder                           |
| `modified:7d`    | Modified within a positive number of hours (`h`), days (`d`) or weeks (`w`) |

Combine different qualifiers, using each at most once: `report kind:pdf modified:7d`. Quote folder paths containing spaces, such as `in:"~/My Documents"`. Backslashes can escape spaces, quotes and backslashes in qualifier values. Unknown colon prefixes remain ordinary search text.

Folder scopes resolve symlinks and require an existing directory. A missing path or a path to a file returns no results.

Valid qualifiers appear as inline chips without changing the query text. Backspace deletes a whole chip, and Undo restores it. Copy and query recall retain the raw query. Incomplete or invalid qualifiers show a syntax hint and return no results. File-name search accepts qualifiers without ordinary terms; content search still requires a word to search for.

Both modes search the Spotlight index, so they find only what Spotlight has indexed.

You can also paste a complete query such as `in release notes` or `d serendipity` into app search. Dupin accepts the same keywords and configured aliases, switches mode, and leaves the search term in the input.

[Folder browsing](./folder-browsing.md) · [File actions](./file-actions.md)

## File ranking

```toml
file_limit = 200 # the most files a Spotlight search shows
file_demotions = ["~/Library", "Caches", "node_modules", ".git", "target", "build", "dist"]
```

File-name search gathers up to four times `file_limit` candidates, ranks them, then applies the display limit. Both limits are capped at 10,000; zero displays no results. Better candidates can replace early results while the search runs. Selecting a row or opening its actions keeps that file available even if it falls below the normal display cut.

`file_demotions` lowers the rank of noisy directories without excluding them. Absolute paths and `~/` paths match directory prefixes; relative entries match directory-component sequences, ignoring case. Entries do not match file names, and an empty list disables these demotions. Edits reload automatically and refresh an active file-name search. Content search keeps Spotlight's order and does not use these rules.
