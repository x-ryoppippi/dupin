# Snippets

Snippets are saved pieces of text that Dupin copies or pastes, with the date, time or clipboard filled in.

## Open it

`snip` <kbd>Space</kbd>, then type to narrow by name or keyword.

## What you see

- A "Snippets" chip.
- **Add Snippet** as the first row.
- Each snippet's name, its keyword and the start of its text; a snippet from `config.toml` is tagged **config.toml**.
- Snippets you use often ranked higher, as apps are.

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/snippets/list-dark.png" />
<img alt="The snippets list: Add Snippet, then Home address, Quote the clipboard, Email signature and Checked stamp, each with its keyword, the start of its text and a config.toml tag." src="../images/snippets/list-light.png" width="680" />
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/snippets/clip-dark.webp" />
<img alt="Typing snip and Space lists the snippets; ↓ moves through them and typing sig leaves Email signature." src="../images/snippets/clip-light.webp" width="680" />
</picture>

## What you can do

| Key / action                                 | What it does                                                                                                                 |
| -------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| <kbd>↩</kbd>                                 | Copy the text, placeholders filled in, and close the panel; with **Paste snippets** on, paste it into the app you were using |
| <kbd>⌘↩</kbd>                                | With **Paste snippets** on, only copy it                                                                                     |
| <kbd>→</kbd>, <kbd>⌘K</kbd> or <kbd>⌃K</kbd> | Show its actions                                                                                                             |
| **Paste into Front App**                     | Paste it, when **Paste snippets** is on                                                                                      |
| **Copy to Clipboard**                        | Copy it                                                                                                                      |
| **Edit Snippet**, **Delete Snippet**         | For a snippet you added: change it, or delete it after a confirmation                                                        |
| **Add Snippet**                              | Open the form for a new snippet; every row offers it                                                                         |
| <kbd>⌫</kbd> on an empty query               | Return to app search                                                                                                         |

Other keys: [Keys](../usage.md#keys).

### Adding and editing

**Add Snippet** opens a form with a name, a keyword and the text. Settings → Snippets lists every snippet and adds, edits and deletes the ones stored in Dupin there too.

| Key / action                | What it does                       |
| --------------------------- | ---------------------------------- |
| <kbd>⇥</kbd>, <kbd>⇧⇥</kbd> | Move to the next or previous field |
| <kbd>↩</kbd> in the text    | Start a new line                   |
| <kbd>⌘↩</kbd>               | Save                               |
| <kbd>Esc</kbd>              | Leave without saving               |

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/settings/snippets-dark.png" />
<img alt="Settings → Snippets: an Add Snippet button above Home address, Quote the clipboard, Email signature and Checked stamp, each with its keyword and text and marked From config.toml." src="../images/settings/snippets-light.png" width="760" />
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/snippets/add-dark.webp" />
<img alt="Add Snippet opens a form; typing a name, Tab, a keyword, Tab and the text, then ⌘Return saves the new snippet." src="../images/snippets/add-light.webp" width="680" />
</picture>

## Notes

- Snippets don't expand as you type in other apps.
- Pasting needs Accessibility access. Turning on **Paste snippets** in Settings → Search requests it when needed; with `snippet_paste` set in `config.toml`, Dupin requests it on the first paste that needs it. While access is unavailable, Dupin copies the text.
- Dupin stores the snippets you add in its [encrypted storage](./storage.md), never in `config.toml`. They sync to your other Macs while [sync](./sync.md) is on.
- A keyword another snippet has, in storage or in `config.toml`, is refused with a message under the field.
- The launcher's form holds up to 16 KiB of text; a longer snippet is edited in Settings, and its row offers no Edit Snippet.

## Configuration

`config.toml` can declare snippets too, as a read-only source: Dupin lists them beside the stored ones, and neither Settings nor the launcher changes them. Each entry under `snippets` is keyed by a short keyword and holds the text to copy, with an optional display name.

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

| Key                       | Default    | Meaning                                                                                                                                                         |
| ------------------------- | ---------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `snippet_keywords`        | `["snip"]` | Aliases for Snippets search, under the same rules as [`dictionary_keywords`](./dictionary.md#configuration), beside any in [`[aliases]`](./keywords.md#aliases) |
| `snippet_paste`           | `false`    | <kbd>↩</kbd> also pastes the snippet into the app you were using by pressing <kbd>⌘V</kbd>. Settings → Search → **Paste snippets**                              |
| `snippets.<keyword>.name` |            | Optional display name, up to 80 characters                                                                                                                      |
| `snippets.<keyword>.text` |            | Required text, up to 64 KiB                                                                                                                                     |

| Placeholder   | Becomes                                             |
| ------------- | --------------------------------------------------- |
| `{date}`      | The local date, as `2026-01-31`                     |
| `{time}`      | The local time on a 24-hour clock, as `09:05`       |
| `{datetime}`  | Both, as `2026-01-31 09:05`                         |
| `{clipboard}` | The clipboard's text, or nothing when it holds none |

Placeholders are filled in when the snippet is used, not when it is listed. Any other braces are ordinary text, so code snippets need no escaping. Dupin reads the clipboard only when the snippet uses `{clipboard}`.

<details>
<summary>Limits</summary>

Keywords are case-sensitive, up to 32 bytes, without spaces. Names are up to 80 characters; text is required and up to 64 KiB. Dupin lists up to 10,000 snippets, stored and `config.toml`'s together, whose keywords, names and text together hold up to 1 MiB. Invalid snippets, and those past either limit in keyword order, are skipped with a notice in the panel; the rest still work. Stored snippets are checked by the same rules. Edits apply live, including to an open snippet search.

</details>

<details>
<summary>Edge cases</summary>

- When there is no snippet at all, the Snippets command isn't suggested as you type a keyword and `?` doesn't list it, so an unused feature stays out of the way. Its keywords still work: `snip`, then Space, shows **Add Snippet**, where you add the first one.
- While access for pasting is unavailable, later attempts also explain how to grant access. Settings → Search shows an Open button for System Settings → Privacy & Security → Accessibility.

</details>

<details>
<summary>config.toml and stored snippets</summary>

When `config.toml` and storage both have a snippet with the same keyword, `config.toml`'s is listed and used, and a notice names the keyword: the file is the declared base, which a stored snippet never silently replaces. Rename the stored one in Settings → Snippets. [Sync](./sync.md) keeps the same rule: `config.toml`'s wins on its own Mac and is never synced.

Dupin never writes `config.toml`'s snippets, whether the file is writable or one Home Manager makes read-only: Settings → Snippets lists them as they are, and to store one in Dupin instead, add it there and delete it from the file.

</details>

<details>
<summary>How it works</summary>

Snippets are the [bundled extension](./extensions.md#bundled-extensions) `snippets`, a TypeScript package in [`extensions/snippets`](../../extensions/snippets) that ships inside Dupin.app. It answers the `snip` keyword and your `snippet_keywords`. Settings → Extensions lists it as a built-in command, with where it comes from, its package switch and its grants, as [Overriding a built-in](./extensions.md#overriding-a-built-in) describes.

The package reads neither `config.toml` nor storage: Dupin checks the snippets as above and hands the valid ones, each with where it comes from (`"source": "config"` or `"stored"`), and `snippet_paste`, to the shipped package as its read-only [`ctx.preferences`](./extension-api.md#itemsctx). The package lists them once with [`items()`](./extension-api.md#itemsctx), and again only when they change; Dupin matches what you type against each name and keyword itself, so typing runs no extension code.

Each row's copy and paste actions are [templates](./extension-api.md#templates): Dupin fills the placeholders when you choose one, so the extension never sees the time or your clipboard. Pasting goes through the extension's `clipboard:paste` scope and editing through its `snippets` scope, which the shipped package holds from the start; Accessibility access stays Dupin's to ask for, as above.

</details>

<details>
<summary>Storage</summary>

Stored snippets live in the `snippets` namespace of Dupin's [encrypted storage](./storage.md), the one the [sync design](../design/sync.md#snippets) names, one record per snippet keyed by its keyword, which [sync](./sync.md) carries while it is on, the later edit of a keyword winning.

Dupin reads them on a worker thread when it starts, and only once something was stored there, so a Mac without stored snippets never reads the storage key for them. The form's Save and the Delete action call the [`snippets`](./extension-permissions.md#snippets) capability, which only an action you chose may use; Dupin checks and stores the snippet, hands the package the new list before the action answers, and the list shows it at once.

</details>

<details>
<summary>Changing it</summary>

To change how snippets are listed, copy the package into the extensions folder and edit it:

```sh
dupin ext eject snippets
```

While the extensions folder is on, your copy answers the snippet keywords in place of the bundled one, as [Overriding a built-in](./extensions.md#overriding-a-built-in) describes. Once it differs from the bundled package it is a third-party extension: Dupin no longer hands it your snippets or fills its templates, and it must be allowed to paste, and to change your stored snippets, before Dupin asks macOS for Accessibility access or stores anything for it. Delete the copy to go back. `dupin ext test` replays the package's [fixtures](./extension-testing.md#fixtures) in `extensions/snippets/fixtures`, whose `snippets` calls change the snippets they hand the package as Dupin's do.

</details>
