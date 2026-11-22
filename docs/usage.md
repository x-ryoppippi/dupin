# Using Dupin

Dupin lives in the background with no Dock icon or menu bar. Its hotkey (⌘Space by default; see [configuration](./configuration.md)) shows the panel; pressing it again, Esc, or clicking elsewhere hides it.

## Keys

| Key                              | In the panel                                          |
| -------------------------------- | ----------------------------------------------------- |
| Hotkey                           | Show or hide the panel                                |
| Type                             | Fuzzy-search installed apps                           |
| Space on an empty query          | Search files with Spotlight, by file and display name |
| `in` then Space                  | Search inside files with Spotlight, as Alfred's `in`  |
| `define` or `d`, then Space      | Look up a word in macOS dictionaries                  |
| ⌫ on an empty query              | Leave file, content or dictionary search              |
| ↑ ↓                              | Move the selection                                    |
| ↑ on an empty query, then ↑ ↓    | Recall searches in the current mode                   |
| ↩                                | Open the app or file                                  |
| ⌘↩                               | Reveal the file in Finder                             |
| → at the end of the input, or ⌘K | Show actions for the selected result                  |
| Esc, or clicking elsewhere       | Hide the panel                                        |
| ⌘Q                               | Quit Dupin                                            |
| ⌘,                               | Open Settings                                         |

Completed searches are remembered for the current Dupin process when you clear
the input, hide the panel or activate a result. With an empty input, ↑ recalls the
latest term in the current mode; further ↑ and ↓ walk through those terms and
back to the empty input. Editing a recalled term restores normal result selection.
This history holds up to 100 terms and is separate from app-ranking history.

## Apps

Dupin finds the `.app` bundles in `/Applications`, `/System/Applications` and `~/Applications`, one folder deep (so `Utilities` is included). An app matches by any name macOS knows it by:

- the name Finder shows, localized (システム設定 for System Settings on a Japanese Mac);
- its file name, when that differs;
- Spotlight's alternate names and keywords: ChatGPT answers to "Codex", its former name, System Settings to "System Preferences", Safari to "browser".

When an app matches through one of those other names, the name shows before its folder in the result.

## Files

Space on an empty query switches to file search; a "Files" chip shows the mode. Every word of the query must appear in the file name or display name. Results appear as Spotlight finds them, then improve as more candidates arrive. Ranking combines filename match quality, your usage and previous query choices, last-used and modified dates, and path depth. Cache and build directories are demoted by the configurable [file ranking rules](./configuration.md#file-ranking).

The list shows up to `file_limit` results (200 by default) from a bounded candidate pool. A selected file or an open action menu keeps its target while results reorder.

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
Right Arrow at the end of an unselected input, or Cmd+K, opens the selected
result's action menu: Open, Reveal in Finder and Copy path. Escape cancels the
menu and returns to the search. The same menu is available for app results.

`define` or `d` followed by Space switches to Dictionary Services ("Define"). Type a word to see its definition from the dictionaries enabled in Dictionary.app. Right Arrow, Cmd+K, Enter or the Actions link opens its menu; choose Open in Dictionary to launch the external app. Backspace on an empty query returns to app search. Set `dictionary_keywords` in [configuration](./configuration.md) to choose other aliases.

You can also paste a complete query such as `in release notes` or `d serendipity` into app search. Dupin accepts the same keywords and configured aliases, switches mode, and leaves the search term in the input.
