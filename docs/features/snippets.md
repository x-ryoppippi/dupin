# Snippets

Type `snip`, then Space, to list your [snippets](#configuration) ("Snippets"); type to narrow them by name or keyword. Each row shows the snippet's name, its keyword and the start of its text. Enter copies the text, with any date, time or clipboard placeholders filled in, and closes the panel; paste it wherever you like. Backspace on an empty query returns to app search.

With pasting turned on in Settings → Search, Enter pastes the snippet into the app you were using and ⌘Enter only copies it. The actions screen (Right Arrow or ⌘K) offers both. Turning on **Paste snippets** in Settings requests Accessibility access when needed. Snippets don't expand as you type in other apps.

Snippets you use often rank higher, as apps do.

## Configuration

Each entry under `snippets` is keyed by a short keyword and holds the text to copy, with an optional display name. Type `snip`, then Space, to list and search them (see [usage](#snippets)); `snippet_keywords` sets other aliases under the same rules as [`dictionary_keywords`](./dictionary.md#configuration).

```toml
snippet_keywords = ["snip"] # type any keyword, then Space, to search snippets
snippet_paste = false # whether Enter on a snippet also pastes it into the front app

[snippets.sig]
name = "Email signature"
text = """
Best wishes,
Sherlock"""

[snippets.stamp]
text = "Checked on {datetime}"

[snippets.quote]
name = "Quote the clipboard"
text = "> {clipboard}"
```

Placeholders are filled in when the snippet is used, not when it is listed:

| Placeholder   | Becomes                                             |
| ------------- | --------------------------------------------------- |
| `{date}`      | The local date, as `2026-01-31`                     |
| `{time}`      | The local time on a 24-hour clock, as `09:05`       |
| `{datetime}`  | Both, as `2026-01-31 09:05`                         |
| `{clipboard}` | The clipboard's text, or nothing when it holds none |

Any other braces are ordinary text, so code snippets need no escaping. Dupin reads the clipboard only when the snippet uses `{clipboard}`.

Keywords are case-sensitive, up to 32 bytes, without spaces. Names are up to 80 characters; text is required and up to 64 KiB. Invalid snippets are skipped with a notice in the panel; the rest still work. Edits apply live, including to an open snippet search.

With `snippet_paste = true`, Enter also pastes the snippet into the app you were using by pressing ⌘V. If you enabled it through `config.toml`, Dupin requests Accessibility access on the first paste that needs it. While access is unavailable, Dupin copies the text. Later attempts also explain how to grant access. Settings → Search shows an Open button for System Settings → Privacy & Security → Accessibility.
