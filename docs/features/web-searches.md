# Web searches

Type `g rust lifetimes`, `gh dupin` or `yt piano` to search Google, GitHub or YouTube. The search appears as the first result alongside any matching apps; Enter opens it in the default browser. Typing just the keyword shows the search too. Select it and press Tab to add a space before typing the query.

Typing a query sends no network request. Dupin encodes the query into the URL when it builds the result and opens the browser or Maps only on activation. Web results do not count towards usage ranking. Query recall behaves as it does for other app-mode searches.

Set `web_searches` and optional `web_fallbacks` in [configuration](#configuration). Fallback searches appear only after ordinary providers finish without results. They are disabled by default. File and Dictionary modes keep their own providers.

[Apple Maps](./maps.md)

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

Keywords contain up to 32 ASCII letters, digits, underscores or hyphens, and are case-sensitive. `in`, configured Dictionary aliases and snippet aliases such as the default `snip` take precedence over web searches. `cal` and `clip` also take precedence when Calendar search and clipboard history are enabled, respectively. Invalid or conflicting web entries are skipped with a notice; valid entries still work. Optional `web_fallbacks` lists keywords in display order (up to 20), shown only when app-mode providers finish without a match. It defaults to `[]`. Changes apply live to the active search; no browser is opened by a configuration reload.
