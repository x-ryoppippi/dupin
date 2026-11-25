# Dictionary

`define` or `d` followed by Space switches to Dictionary Services ("Define"). Type a word to see its definition from the dictionaries enabled in Dictionary.app. Dupin lays the entry out from Dictionary.app's text: the pronunciation beside a label; each part of speech (noun, verb, …) under a subheading with its numbered senses (1 2 3 or ① ② ③) as a numbered list; the sub-senses after each `•` nested under their sense; examples after a sense's colon or a `▸` in italics; antonyms labeled in bold; and parts such as Phrases and Origin under their own headings. Text in any other layout shows as paragraphs, so nothing is left out. A word no dictionary has shows No definition found. Backspace on an empty query returns to app search. Set `dictionary_keywords` in [configuration](#configuration) to choose other aliases.

Enter opens the word in Dictionary.app. Right Arrow, Cmd+K or the Actions link opens its action screen, with **Open in Dictionary**, **Copy Definition**, which copies the definition as the dictionary wrote it, and Send to….

## The bundled extension

Dictionary is the [bundled extension](./extensions.md#bundled-extensions) `dictionary`, in TypeScript in the repository's [`extensions/dictionary`](../../extensions/dictionary). It looks each word up with the extension [`dictionary` capability](./extensions.md#dictionary), which reads Dictionary Services and lays the entry out in Dupin, and shows the entry's Markdown as the row's detail. It answers Dictionary's own command, so `dictionary_keywords` choose its keywords, the `keyword` in its `package.json` is never used, and Settings → Extensions lists it as a built-in command, with where it comes from, its package switch and its grant. `define` answers only while that package switch is on; with it off, Dictionary says it isn't available. If it can't load, Dictionary's keywords say so instead of searching. To change it, copy it into the extensions folder and edit the copy:

```sh
dupin ext eject dictionary
```

While the extensions folder is on, your copy answers Dictionary's keywords in place of the bundled one, as [Overriding a built-in](./extensions.md#overriding-a-built-in) describes. Once it differs from the bundled package, it is a third-party extension: the panel asks you to turn it on and to allow it to look words up before it shows any definition. Its fixtures run with `dupin ext test ~/.config/dupin/extensions/dictionary`.

## Configuration

Set `dictionary_keywords` to a list of aliases for Dictionary search. The defaults are `define` and `d`; add, remove or replace entries to suit your workflow. Valid edits take effect without relaunching.

```toml
dictionary_keywords = ["define", "d"] # type any keyword, then Space, to look up a word
```

Aliases must be non-empty, have no leading or trailing whitespace and leave other commands' keywords, such as `in`, available. Dupin reports invalid or conflicting aliases in the panel and ignores those entries; the other aliases still work.
