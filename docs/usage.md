# Using Dupin

Dupin lives in the background with no Dock icon or menu bar. Its hotkey (⌘Space by default; see [configuration](./configuration.md)) shows the panel; pressing it again, Esc, or clicking elsewhere hides it.

## Keys

| Key                           | In the panel                                          |
| ----------------------------- | ----------------------------------------------------- |
| Hotkey                        | Show or hide the panel                                |
| Type                          | Fuzzy-search installed apps                           |
| Space on an empty query       | Search files with Spotlight, by file and display name |
| `in` then Space               | Search inside files with Spotlight, as Alfred's `in`  |
| `define` or `d`, then Space   | Look up a word in macOS dictionaries                  |
| ⌫ on an empty query           | Leave file, content or dictionary search              |
| ↑ ↓                           | Move the selection                                    |
| ↑ on an empty query, then ↑ ↓ | Recall searches in the current mode                   |
| ↩                             | Open the app or file                                  |
| ⌘↩                            | Reveal the file in Finder                             |
| Esc, or clicking elsewhere    | Hide the panel                                        |
| ⌘Q                            | Quit Dupin                                            |
| ⌘,                            | Open Settings                                         |

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

Space on an empty query switches to file search; a "Files" chip shows the mode. Every word of the query must appear in the file name or display name. Results stream in as Spotlight finds them, up to `file_limit` (200 by default).

`in` followed by Space switches to content search ("In files"). Every word must start a word in the text Spotlight indexed from inside the file.

Both modes search the Spotlight index, so they find only what Spotlight has indexed.

`define` or `d` followed by Space switches to Dictionary Services ("Define"). Type a word to see its definition from the dictionaries enabled in Dictionary.app. Enter opens Dictionary.app to that term. Backspace on an empty query returns to app search. Set `dictionary_keywords` in [configuration](./configuration.md) to choose other aliases.

You can also paste a complete query such as `in release notes` or `d serendipity` into app search. Dupin accepts the same keywords and configured aliases, switches mode, and leaves the search term in the input.
