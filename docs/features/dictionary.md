# Dictionary

`define` or `d` followed by Space switches to Dictionary Services ("Define"). Type a word to see its definition from the dictionaries enabled in Dictionary.app. Right Arrow, Cmd+K, Enter or the Actions link opens its action screen; choose Open in Dictionary to launch the external app. Backspace on an empty query returns to app search. Set `dictionary_keywords` in [configuration](#configuration) to choose other aliases.

## Configuration

Set `dictionary_keywords` to a list of aliases for Dictionary search. The defaults are `define` and `d`; add, remove or replace entries to suit your workflow. Valid edits take effect without relaunching.

```toml
dictionary_keywords = ["define", "d"] # type any keyword, then Space, to look up a word
```

Aliases must be non-empty, have no leading or trailing whitespace and leave other commands' keywords, such as `in`, available. Dupin reports invalid or conflicting aliases in the panel and ignores those entries; the other aliases still work.
