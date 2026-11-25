# Keywords

A keyword typed at the start of the main search, then Space, enters a command's own search: `cal` for [Calendar](./calendar.md), `define` or `d` for [Dictionary](./dictionary.md), `in` for [file contents](./files.md), `clip` for [clipboard history](./clipboard.md), `snip` for [snippets](./snippets.md), `emoji` for [Emoji & Symbols](./emoji.md), `help` for Dupin's own links, and an [extension's](./extensions.md) keyword for that extension. A [web search's](./web-searches.md) alias, such as `g` or `gh`, takes the query that follows it, and Space on an empty query searches [files](./files.md).

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

Every keyword reaches one command or web search. Dupin registers them in this order, and a keyword already taken is skipped with a notice that names the setting to fix:

1. Built-in commands: `in`, and `cal` and `clip` while Calendar search and clipboard history are on.
2. `dictionary_keywords` and `snippet_keywords` in config.toml.
3. Web search aliases, from `web_searches` or the defaults `g`, `gh`, `maps` and `yt`.
4. Extension keywords, bundled extensions first, then the extensions folder in order.

`dupin ext install` refuses a package whose keyword one of these already holds.
