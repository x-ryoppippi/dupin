# Tutorial

Five minutes with Dupin, from the first search to your own shortcuts.

## Open an app

Press ⌘Space, type a few letters of an app's name and press ↩.

<picture>
<source media="(prefers-color-scheme: dark)" srcset="images/usage/search-dark.png" />
<img alt="The Dupin panel with “ma” typed: Maps first, then the Search Apple Maps web search, Mail, Magnifier and other apps." src="images/usage/search-light.png" width="680" />
</picture>

Esc, or ⌘Space again, hides the panel. Dupin learns what you pick, so the same letters find it first next time.

## Find a file

Press Space on an empty query, then type part of a file's name. ↩ opens it, ⌘↩ reveals it in Finder and → shows what else you can do with it. Type `in` and Space instead to search inside files.

## Use a keyword

A keyword and Space enters a command:

| Type            | And you get                                                                                                |
| --------------- | ---------------------------------------------------------------------------------------------------------- |
| `d serendipity` | The word in the macOS dictionaries                                                                         |
| `cal`, Space    | Your upcoming events, once Calendar search is on in Settings → Search ([Calendar](./features/calendar.md)) |
| `clip`, Space   | What you copied, once clipboard history is on in Settings → Search ([Clipboard](./features/clipboard.md))  |
| `g rust`        | A Google search for “rust” ([Web searches](./features/web-searches.md))                                    |

⌫ on an empty query leaves the command.

## See every keyword

Type `?`.

<picture>
<source media="(prefers-color-scheme: dark)" srcset="images/keywords/list-dark.png" />
<img alt="The panel with “?” typed, listing commands with their keywords as tags." src="images/keywords/list-light.png" width="680" />
</picture>

## Make it yours

⌘, opens Settings. Everything there is also in `~/.config/dupin/config.toml` ([Configuration](./configuration.md)). Add your own shorthands with `[aliases]`:

```toml
[aliases]
c = "calendar" # c, then Space, enters Calendar search
ff = "app:org.mozilla.firefox" # ff shows Firefox first
```

And give a command a global shortcut of its own with `[bindings]`, here one that opens the panel in clipboard history:

```toml
[bindings."cmd+shift+v"]
target = "command"
command = "dupin open --query 'clip '"
```

## Next

- [Using Dupin](./usage.md): every key in the panel
- [Keywords](./features/keywords.md) and [Hotkeys](./features/hotkeys.md)
- [Extensions](./features/extensions.md): write your own in TypeScript
- [Philosophy](./philosophy.md): why Dupin works the way it does
