# Keywords

A keyword typed at the start of the main search, then Space, enters a command's own search: `cal` for [Calendar](./calendar.md), `define` or `d` for [Dictionary](./dictionary.md), `in` for [file contents](./files.md), `clip` for [clipboard history](./clipboard.md), `snip` for [snippets](./snippets.md), `emoji` for [Emoji & Symbols](./emoji.md), `snap` for [snapshots](./snapshots.md), `help` for Dupin's own links, and an [extension's](./extensions.md) keyword for that extension. A [web search's](./web-searches.md) alias, such as `g` or `gh`, takes the query that follows it, and Space on an empty query searches [files](./files.md).

## Finding them as you type

While what you type in the main search is a keyword, or the start of one, the command it reaches shows as a row, with the keyword and `␣` (Space) as a tag at its end, such as **Calendar** `cal ␣`. Enter or Tab on that row enters the command exactly as typing the keyword and Space does; for a web search it leaves the alias and a space in the query. The rows a keyword reaches directly, such as a web search's, show the keyword as a tag too.

The row never pushes the best match down for a short query:

- An exact keyword of three or more characters, such as `cal` or `define`, leads the results.
- A shorter exact keyword, such as `d` or `in`, leads only when no other command's keyword starts with it; `g`, which `gh` also starts, does not.
- The start of a keyword, such as `ca` or `e`, follows the first result, with up to three commands, exact and shortest keywords first.
- A web search's own alias already shows its search first, so it adds no second row.

Type `?` alone to list every command, web search and the files search with their keywords; choose one to enter it. An empty query shows what it always did.

Keywords are case-sensitive when typed with Space; the rows match them in any case, and Enter or Tab types them as configured.

## Who gets a keyword

Every keyword reaches one command, web search, app or file. Dupin registers them in this order, and a keyword already taken is skipped with a notice that names the setting to fix:

1. Built-in commands: `in`, and `cal` and `clip` while Calendar search and clipboard history are on.
2. `dictionary_keywords` and `snippet_keywords` in config.toml.
3. `snap` for snapshots.
4. Web search aliases, from `web_searches` or the defaults `g`, `gh`, `maps` and `yt`.
5. [Aliases](#aliases) in `[aliases]`.
6. Extension keywords, bundled extensions first, then the extensions folder in order.

So an alias never takes a keyword a built-in, `dictionary_keywords`, `snippet_keywords` or a web search already has, and an extension's keyword never takes an alias. `dupin ext install` refuses a package whose keyword one of these already holds.

## Aliases

`[aliases]` in config.toml maps a shorthand you type in the main search to what it reaches:

```toml
[aliases]
c = "calendar" # c, then Space, enters Calendar search, as cal does
lookup = "dictionary"
wc = "word-count" # an extension, by its package name
ff = "app:org.mozilla.firefox" # typing ff shows Firefox first
goo = "web:g" # goo rust searches Google, as g rust does
dl = "file:~/Downloads" # typing dl shows the Downloads folder first
```

| Target            | Reaches                                                                                                                                                                             |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A command's name  | `files`, `contents`, `calendar`, `clipboard`, `dictionary`, `snippets`, `snapshots`, or an extension's package name. The alias, then Space, enters the command as its keyword does. |
| `app:<bundle id>` | An installed app, such as `app:com.apple.Safari`. Typing the alias shows the app as the first result; Enter opens it.                                                               |
| `web:<keyword>`   | A [web search](./web-searches.md) by its keyword. The alias takes a query as that keyword does, and is listed beside it.                                                            |
| `file:<path>`     | A file or folder by an absolute or `~/` path. Typing the alias shows it as the first result; Enter opens it.                                                                        |

An alias is 1 to 32 ASCII letters, digits, underscores or hyphens, and config.toml takes up to 64. A malformed alias or target makes config.toml invalid, naming the entry and what is wrong, and a reload keeps the previous configuration. An alias that is well formed but reaches nothing is left out with a notice, and Settings → Keywords says why beside it: its keyword is taken, its command's feature is off, its extension isn't installed or on, no web search answers to its keyword, or its app or file isn't there. The rows a typed alias reaches show it as a tag, and `?` lists aliases with each command's keywords.

`dictionary_keywords` and `snippet_keywords` keep working as before, beside any alias for the same command.

## Settings

Settings → Keywords lists every command, web search, app and file a keyword reaches with its keywords, the aliases among them drawn quieter, and each extension with its keyword. Below that it lists your aliases, each with what it reaches and, if it is left out, why, with Remove, and fields to add one. The last card edits `dictionary_keywords` and `snippet_keywords`, separated by spaces. A key set in a read-only config.toml, such as one Home Manager writes, is shown locked. With Home Manager, set them in `programs.dupin.settings.aliases`.
