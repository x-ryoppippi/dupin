# Tutorial

Five minutes with Dupin, from the first search to your own shortcuts.

## Open an app

Press ⌘Space, type a few letters of an app's name and press ↩.

<picture>
<source media="(prefers-color-scheme: dark)" srcset="images/tutorial/apps-dark.webp" />
<img alt="Typing “maps” narrows the apps to Maps; ⌘K then shows its actions: Open, Reveal in Finder, Quick Look and Send to." src="images/tutorial/apps-light.webp" width="680" />
</picture>

⌘K shows what else you can do with the selected row. Esc, or ⌘Space again, hides the panel. Dupin learns what you pick, so the same letters find it first next time.

## Find a file

With nothing typed, the panel lists what you opened recently. Tab on a folder lists what is in it, and typing filters it.

<picture>
<source media="(prefers-color-scheme: dark)" srcset="images/tutorial/files-dark.webp" />
<img alt="Recent items with the Reports folder first; Tab lists the folder, and typing “ann” leaves Annual report 2026.pdf." src="images/tutorial/files-light.webp" width="680" />
</picture>

Press Space on an empty query to search every file by name with Spotlight, or type `in` and Space to search inside files. ↩ opens a file and ⌘↩ reveals it in Finder.

To peek without opening, press ⌘Y on a file or an app: Quick Look shows it, and Esc brings you back to the list with your search and selection as they were ([File actions](./features/file-actions.md)).

## Use a keyword

A keyword and Space enters a command:

| Type            | And you get                                                                                                |
| --------------- | ---------------------------------------------------------------------------------------------------------- |
| `d serendipity` | The word in the macOS dictionaries                                                                         |
| `cal`, Space    | Your upcoming events, once Calendar search is on in Settings → Search ([Calendar](./features/calendar.md)) |
| `clip`, Space   | What you copied, once clipboard history is on in Settings → Search ([Clipboard](./features/clipboard.md))  |
| `g rust`        | A Google search for “rust” ([Web searches](./features/web-searches.md))                                    |

<picture>
<source media="(prefers-color-scheme: dark)" srcset="images/tutorial/dictionary-dark.webp" />
<img alt="Typing “d” and Space enters the Define mode; typing “serendipity” shows its pronunciation, meaning and origin." src="images/tutorial/dictionary-light.webp" width="680" />
</picture>

⌫ on an empty query leaves the command.

## See every keyword

Type `?`, and choose one to enter it.

<picture>
<source media="(prefers-color-scheme: dark)" srcset="images/tutorial/keywords-dark.webp" />
<img alt="Typing “?” lists every command with its keywords as tags: Dictionary (d, define), Dupin (help), Emoji &amp; Symbols (emoji), File Contents (in), Files (Space) and web searches; ↓ moves through them." src="images/tutorial/keywords-light.webp" width="680" />
</picture>

## Make it yours

⌘, opens Settings. Everything there is also in `~/.config/dupin/config.toml` ([Configuration](./configuration.md)). Add your own shorthands with `[aliases]`:

```toml
[aliases]
c = "calendar" # c, then Space, enters Calendar search
ff = "app:org.mozilla.firefox" # ff shows Firefox first
```

And give a command a global shortcut of its own with `[bindings]`, here ⇧⌘V for clipboard history, or record one in Settings → Keywords:

```toml
[bindings."cmd+shift+v"]
target = "dupin"
command = "clipboard"
```

## Next

- [Using Dupin](./usage.md): every key in the panel
- [Keywords](./features/keywords.md) and [Hotkeys](./features/hotkeys.md)
- [Extensions](./features/extensions.md): write your own in TypeScript
- [Philosophy](./philosophy.md): why Dupin works the way it does
