# Dictionary

`define` or `d` followed by Space switches to Dictionary Services ("Define"). Type a word to see its definition from the dictionaries enabled in Dictionary.app. Dupin lays the entry out from Dictionary.app's text: the pronunciation beside a label; each part of speech (noun, verb, …) under a subheading with its numbered senses (1 2 3 or ① ② ③) as a numbered list; the sub-senses after each `•` nested under their sense; examples after a sense's colon or a `▸` in italics; antonyms labeled in bold; and parts such as Phrases and Origin under their own headings. Text in any other layout shows as paragraphs, so nothing is left out. Right Arrow, Cmd+K, Enter or the Actions link opens its action screen; choose Open in Dictionary to launch the external app. Backspace on an empty query returns to app search. Set `dictionary_keywords` in [configuration](#configuration) to choose other aliases.

## Configuration

Set `dictionary_keywords` to a list of aliases for Dictionary search. The defaults are `define` and `d`; add, remove or replace entries to suit your workflow. Valid edits take effect without relaunching.

```toml
dictionary_keywords = ["define", "d"] # type any keyword, then Space, to look up a word
```

Aliases must be non-empty, have no leading or trailing whitespace and leave other commands' keywords, such as `in`, available. Dupin reports invalid or conflicting aliases in the panel and ignores those entries; the other aliases still work.
