# Snippets

Type `snip`, then Space, to list your [snippets](#configuration) ("Snippets"); type to narrow them by name or keyword. Each row shows the snippet's name, its keyword and the start of its text. Enter copies the text, with any date, time or clipboard placeholders filled in, and closes the panel; paste it wherever you like. Backspace on an empty query returns to app search.

With pasting turned on in Settings → Search, Enter pastes the snippet into the app you were using and ⌘Enter only copies it. The actions screen (Right Arrow or ⌘K) offers both. Turning on **Paste snippets** in Settings requests Accessibility access when needed. Snippets don't expand as you type in other apps.

Snippets you use often rank higher, as apps do.

## Configuration

Each entry under `snippets` is keyed by a short keyword and holds the text to copy, with an optional display name. Type `snip`, then Space, to list and search them (see [usage](#snippets)); `snippet_keywords` sets other aliases under the same rules as [`dictionary_keywords`](./dictionary.md#configuration), beside any in [`[aliases]`](./keywords.md#aliases).

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

Keywords are case-sensitive, up to 32 bytes, without spaces. Names are up to 80 characters; text is required and up to 64 KiB. Dupin lists up to 10,000 snippets, whose keywords, names and text together hold up to 1 MiB. Invalid snippets, and those past either limit in keyword order, are skipped with a notice in the panel; the rest still work. Edits apply live, including to an open snippet search.

With `snippet_paste = true`, Enter also pastes the snippet into the app you were using by pressing ⌘V. If you enabled it through `config.toml`, Dupin requests Accessibility access on the first paste that needs it. While access is unavailable, Dupin copies the text. Later attempts also explain how to grant access. Settings → Search shows an Open button for System Settings → Privacy & Security → Accessibility.

## How it works

Snippets are the [bundled extension](./extensions.md#bundled-extensions) `snippets`, a TypeScript package in [`extensions/snippets`](../../extensions/snippets) that ships inside Dupin.app. It answers the `snip` keyword and your `snippet_keywords`. Settings → Extensions lists it as a built-in command, with where it comes from, its package switch and its grant, as [Overriding a built-in](./extensions.md#overriding-a-built-in) describes. The package doesn't read `config.toml`: Dupin checks your snippets as above and hands the valid ones, with `snippet_paste`, to the shipped package as its read-only [`ctx.preferences`](./extensions.md#itemsctx). The package lists them once with [`items()`](./extensions.md#itemsctx), and again only when they change; Dupin matches what you type against each name and keyword itself, so typing runs no extension code.

Each row's actions are [templates](./extensions.md#actions): Dupin fills the placeholders when you choose one, so the extension never sees the time or your clipboard. Pasting goes through the extension's `clipboard:paste` scope, which the shipped package holds from the start; Accessibility access stays Dupin's to ask for, as above.

To change how snippets are listed, copy the package into the extensions folder and edit it:

```sh
dupin ext eject snippets
```

While the extensions folder is on, your copy answers the snippet keywords in place of the bundled one, as [Overriding a built-in](./extensions.md#overriding-a-built-in) describes. Once it differs from the bundled package it is a third-party extension: Dupin no longer hands it your snippets or fills its templates, and it must be allowed to paste before Dupin asks macOS for Accessibility access. Delete the copy to go back. `dupin ext test` replays the package's [fixtures](./extensions.md#fixtures) in `extensions/snippets/fixtures`.
