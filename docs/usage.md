# Using Dupin

Dupin lives in the background with no Dock icon or menu bar. Its hotkey (⌘Space by default; see [configuration](./configuration.md)) shows the panel; pressing it again, Esc, or clicking elsewhere hides it.

## First run

The first launch opens Settings with a short welcome: choose the hotkey, turn on launch at login, and see which keys switch modes. ⌘Space is Spotlight's shortcut too; to give it to Dupin, turn off Show Spotlight search in System Settings → Keyboard → Keyboard Shortcuts → Spotlight, or choose another hotkey. When another app holds the chord you press, Dupin names it.

The welcome appears once. It is skipped when `history.json` already exists in `~/Library/Application Support/Dupin`, as it does for anyone who used Dupin before. **Show Welcome** in the Settings sidebar brings it back.

Settings → Permissions appears when a feature uses a macOS permission. It shows each relevant grant and links to its pane in System Settings. Dupin asks for access only when you use a feature that needs it. File search uses the Spotlight index and needs no grant.

## Search selected text

Set a **Selection hotkey** in Settings → General. Select text in another app and press the shortcut to open Dupin with that text in the query field. Type to replace the selection. If the app exposes no selected text, Dupin opens with an empty query.

Dupin needs macOS Accessibility access for this shortcut. If access is missing, choose **Open System Settings** in the panel and enable Dupin under **Privacy & Security → Accessibility**, then try again.

## Keys

| Key                                     | In the panel                                          |
| --------------------------------------- | ----------------------------------------------------- |
| Hotkey                                  | Show or hide the panel                                |
| Type                                    | Fuzzy-search installed apps                           |
| Space on an empty query                 | Search files with Spotlight, by file and display name |
| `in` then Space                         | Search inside files with Spotlight, as Alfred's `in`  |
| `define` or `d`, then Space             | Look up a word in macOS dictionaries                  |
| `clip` then Space                       | Search the clipboard history, when it is on           |
| ⌫ on an empty query                     | Leave file, content, dictionary or clipboard search   |
| ↑ ↓                                     | Move the selection                                    |
| Tab at the end of an unselected input   | Complete the selected search keyword                  |
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

## Apps

With an empty query, Dupin shows up to ten apps and files from your usage history,
ranked by how recently and how often you used them. These rows have the regular
app and file actions. Turn off **Show recent items** in Settings → Search to keep
the panel empty before typing.

Executable [script commands](./scripts.md) also appear in normal search. Scripts with arguments open a separate argument field; their output can stay inline or stream into the Script Output window.

Dupin finds the `.app` bundles in `/Applications`, `/System/Applications` and `~/Applications`, one folder deep (so `Utilities` is included). An app matches by any name macOS knows it by:

- the name Finder shows, localized (システム設定 for System Settings on a Japanese Mac);
- its file name, when that differs;
- Spotlight's alternate names and keywords: ChatGPT answers to "Codex", its former name, System Settings to "System Preferences", Safari to "browser".

When an app matches through one of those other names, the name shows before its folder in the result.

## Hotkeys

Use a global shortcut to launch an app, bring it to the front, or run a shell command. Set `hide_if_frontmost = true` to hide an app if it is already frontmost. A shell command runs silently in the background, so you can use it to open the panel with a preset query or run a CLI directly. See [hotkey configuration](./configuration.md#hotkeys).

## Clipboard history

Turn on clipboard history in Settings → Search (it is off by default; see [configuration](./configuration.md#clipboard-history)). Then type `clip` and Space: a "Clipboard" chip shows the mode and the list starts with your most recent copies, each with how long ago it was copied. Type to keep only entries containing every word, ignoring case. ↑ and ↓ move through the list even with an empty query. Clipboard search terms are not kept for query recall. Enter copies the selected entry back to the clipboard and hides the panel, ready to paste; file entries copy the files, as Finder's Copy does. Dupin does not paste into the frontmost app for you.

The action screen (→, ⌘K or Ctrl+K) offers Copy, Delete from History and Clear History. Deleting removes that entry only; the clipboard itself keeps whatever it holds. Clear History asks for confirmation with Cancel selected before forgetting every entry. Clipboard results can't be saved as snapshots and are not recorded in usage-ranking history.

## Snapshots

Press ⌘⇧S in the launcher to save a finished calculation, search or extension view.
Open **Snapshots** from Settings, or press ⌘⇧O in the launcher. Its separate,
resizable window searches titles and saved content. Click a title or use ↑/↓ to
read it. Delete opens a confirmation before removing the selected snapshot;
⌘W closes the window. Editing the search field never deletes saved content.

Snapshots show historical content, including the visible result list, without
running saved actions. **Resume search** (⌘R) explicitly runs the original mode
and query again when its command is still available. Removed extensions leave
their saved content readable. Extension-specific JSON state is reserved in the
format; current commands resume through their query.

Snapshots stay locally in Dupin's application-support directory (or
`$XDG_DATA_HOME/dupin/snapshots` when set), with private permissions. There is no
automatic capture or telemetry. Storage accepts up to 100 entries, 512 KiB per
entry and 8 MiB in total; a save that exceeds a limit reports an error. Invalid
or unsupported archives remain on disk and block edits, with the problem shown
in the Snapshots window.

Use `dupin snapshots list --json` for saved Snapshot metadata, `dupin snapshots open
[--id ID]` to show the separate window, and `dupin snapshots close` to hide it.
The list returns the ID, saved time, mode, title and query, and flags archive
problems or truncation. These commands leave the saved archive and usage-ranking
history intact. Opening dismisses the launcher and may retain the completed query
for in-memory recall. An active picker blocks opening.

## Settings

Search for a macOS settings pane, such as `bluetooth`, `displays` or `keyboard`, alongside apps. Enter opens the pane in System Settings. Dupin discovers panes that the installed macOS declares as settings extensions and explicitly allows opening through its settings URL scheme; availability and names follow that macOS version. Searching does not open or change settings.

Type `dupin settings` to open Dupin's own preferences. This uses the same window handoff as ⌘,.

## Web searches

Type `g rust lifetimes`, `gh dupin` or `yt piano` to search Google, GitHub or YouTube. The search appears as the first result alongside any matching apps; Enter opens it in the default browser. Typing just the keyword shows the search too. Select it and press Tab to add a space before typing the query.

Type `maps baker street` and press Enter on the Apple Maps result to search for an address or place in Maps. The Apple Maps keyword needs a search term, so typing just `maps` still finds the Maps app.

Typing a query sends no network request. Dupin encodes the query into the URL when it builds the result and opens the browser or Maps only on activation. Web results do not count towards usage ranking. Query recall behaves as it does for other app-mode searches.

Set `web_searches` and optional `web_fallbacks` in [configuration](./configuration.md). Fallback searches appear only after ordinary providers finish without results. They are disabled by default. File and Dictionary modes keep their own providers.

## Calendar

Enable **Calendar search** in Settings → Search. Type `cal` followed by Space for up to 30 events over the next seven days. To search from 30 days ago to 90 days ahead, type words from the title or location in any order. Each row shows the calendar and the event time in the Mac's local time zone. All-day events show their dates and an “All day” label.

The first search may ask for Full Access to Calendars. If access is missing, select the permission row to open System Settings → Privacy & Security → Calendars. Invoke `cal` again after granting access there. Calendar results stay out of Snapshots and search history.

Press Enter to open the original event in Calendar. Dupin uses an undocumented Calendar link. If Calendar cannot resolve an event or recurring occurrence, it may open without selecting it.

## Snippets

Type `snip`, then Space, to list your [snippets](./configuration.md#snippets) ("Snippets"); type to narrow them by name or keyword. Each row shows the snippet's name, its keyword and the start of its text. Enter copies the text, with any date, time or clipboard placeholders filled in, and closes the panel; paste it wherever you like. Backspace on an empty query returns to app search.

With pasting turned on in Settings → Search, Enter pastes the snippet into the app you were using and ⌘Enter only copies it. The actions screen (Right Arrow or ⌘K) offers both. Pasting needs Accessibility access, which Dupin asks for only when you paste a snippet without it, once per launch; after that, it copies the snippet and explains how to allow access. Snippets don't expand as you type in other apps.

Snippets you use often rank higher, as apps do.

## Files

Space on an empty query switches to file search; a "Files" chip shows the mode. Every word of the query must appear in the file name or display name. Results appear as Spotlight finds them, then improve as more candidates arrive. Ranking combines filename match quality, your usage and previous query choices, last-used and modified dates, and path depth. Cache and build directories are demoted by the configurable [file ranking rules](./configuration.md#file-ranking).

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
Right Arrow at the end of an unselected input, or Cmd+K, opens the selected
result's action screen: Open, Open with…, Reveal in Finder, Quick Look, Copy path, Copy name, Rename…, Move to…, Copy to…, Duplicate, Compress and Move to Trash. Escape cancels the
screen and returns to the exact search and selection. Use ↑ and ↓ to select an action, Enter to run it, and Left or Escape to return. Ctrl+K also opens actions. App results have the same screen with Open, Reveal, Quick Look and Copy path; file-only actions are omitted.

Open with… lists the applications registered for the selected file or folder, with its default handler first. Choose an application to open the item. Left or Escape returns to Actions without changing the query. Move to Trash opens a confirmation with Cancel selected; choose Move to Trash explicitly to continue. Held Enter cannot confirm. After success the file leaves the current result list. Failures appear in the launcher. Folder browsing inside the picker is not supported.

Rename… puts the item's name in the input, with the part before its extension selected, as Finder does. Type the new name and press Enter to rename it in the same folder, or Escape to return to the search unchanged. A name that is empty, `.` or `..`, contains `/` or `:`, or is longer than 255 UTF-16 code units (as APFS counts) shows why and stays editable; Enter does nothing until it is fixed. A held Enter that starts or confirms a rename does not repeat into the next step. Renaming never replaces an existing item: if the name is taken, the rename fails and says so. After success the result shows the new name in place.

Move to… and Copy to… open a Spotlight folder search in the panel. Type a folder name, select a result and press Enter to confirm, or press Escape to return to the original search. The operation runs in the background and reports its outcome in the panel. Move keeps the item's name and refuses an existing name or the item's current folder. Copy uses the original name if available, then Finder-style names such as `report copy.pdf` and `report copy 2.pdf`. A folder cannot be moved or copied into itself or one of its descendants.

When moving between volumes, Dupin saves the copy first. If the source still matches, Dupin puts the original in the Trash so it can be recovered. If the source changed or could not be verified, Dupin keeps it in place and reports the problem.

Duplicate copies the item beside itself as Finder does: `report copy.pdf`, then `report copy 2.pdf`, and so on; duplicating a copy continues its numbering. Compress zips it beside itself with `ditto`, as Finder's Compress does: `report.pdf.zip`, then `report.pdf 2.zip`. Both keep the launcher open, work in the background and report the new name, or the failure, in the launcher. Neither replaces an existing item.

Quick Look opens the native preview; Escape returns to the picker with its search and selection intact.

`define` or `d` followed by Space switches to Dictionary Services ("Define"). Type a word to see its definition from the dictionaries enabled in Dictionary.app. Right Arrow, Cmd+K, Enter or the Actions link opens its action screen; choose Open in Dictionary to launch the external app. Backspace on an empty query returns to app search. Set `dictionary_keywords` in [configuration](./configuration.md) to choose other aliases.

You can also paste a complete query such as `in release notes` or `d serendipity` into app search. Dupin accepts the same keywords and configured aliases, switches mode, and leaves the search term in the input.
