# Web searches

Type `g rust lifetimes`, `gh dupin` or `yt piano` to search Google, GitHub or YouTube. The search appears as the first result alongside any matching apps; Enter opens it in the default browser. Typing just the keyword shows the search too. Select it and press Tab to add a space before typing the query. Type `web` and Space, then a query, to list every web search for it.

Typing a query sends no network request. Dupin encodes the query into the URL when it builds the result and opens the browser or Maps only on activation. Web results do not count towards usage ranking. Query recall behaves as it does for other app-mode searches. Send to… offers each web search for selected text.

Set `web_searches` and optional `web_fallbacks` in [configuration](#configuration). Fallback searches appear only after ordinary providers finish without results. They are disabled by default. File and Dictionary modes keep their own providers.

[Apple Maps](./maps.md)

## The Web Search extension

Web searches are the bundled **Web Search** extension, [`extensions/web-search`](../../extensions/web-search), which lists them as [web search items](./extensions.md#web-searches). Dupin matches the keyword and fills the URL itself, so typing never waits on the extension. Turn it off in Settings → Extensions to turn every web search off. When another command, such as a Dictionary or snippet alias or a `web` entry in `web_searches`, uses its keyword `web`, Settings says so and only typing `web` to list every search stops working.

`dupin ext eject web-search` copies it into the extensions folder. An unchanged copy behaves as the bundled one. Once you change it, it is a third-party extension: it no longer receives `web_searches` or `web_fallbacks` from config.toml, and its web searches are listed only under its keyword `web`. Only the bundled package, or an unchanged copy, adds aliases and fallbacks to the main search and searches to Send to, so no other extension can put a search there that receives what you type.

## Configuration

The default keywords are `g` (Google), `gh` (GitHub), `maps` (Apple Maps) and `yt` (YouTube). An explicit `web_searches` table replaces the defaults. Add the Maps entry below to use it with a custom table, or set `web_searches = {}` to disable all web searches.

Each entry has a display name and an HTTP or HTTPS URL template with exactly one `{query}` in its path, query or fragment. The exact template `maps://?q={query}` opens Apple Maps directly. Dupin encodes the text as UTF-8 URL data. Templates cannot contain credentials or put the placeholder in the origin.

```toml
web_fallbacks = ["gh"]

[web_searches.gh]
name = "GitHub"
url = "https://github.com/search?q={query}"

[web_searches.maps]
name = "Apple Maps"
url = "maps://?q={query}"

[web_searches.docs]
name = "Rust documentation"
url = "https://doc.rust-lang.org/std/?search={query}"
```

Keywords contain up to 32 ASCII letters, digits, underscores or hyphens, and are case-sensitive. `in`, configured Dictionary aliases and snippet aliases such as the default `snip` take precedence over web searches, and web searches over extension keywords. `cal` and `clip` also take precedence when Calendar search and clipboard history are enabled, respectively. Invalid or conflicting web searches, the default ones included, and fallbacks without one are skipped with a notice; valid entries still work. A name may not contain `{query}`. Optional `web_fallbacks` lists keywords in display order (up to 20), shown only when app-mode providers finish without a match. It defaults to `[]`. Changes apply live to the active search; no browser is opened by a configuration reload.
