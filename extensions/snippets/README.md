# Snippets

Copies or pastes saved text with the date, time or clipboard filled in, and adds, edits and deletes stored snippets.

| Keyword       | `snip` (`snippet_keywords`)                 |
| ------------- | ------------------------------------------- |
| Command       | `snippets`                                  |
| On by default | Yes                                         |
| User docs     | [Snippets](../../docs/features/snippets.md) |

A port of the former native Snippets; see [Built-ins as bundled extensions](../../docs/design/extension-platform.md#built-ins-as-bundled-extensions). It answers the built-in command under that command's keywords.

## How it's built

The bundled example of a form.

- **`items(ctx)`**: **Add Snippet**, then every snippet Dupin hands over in `ctx.preferences.snippets`, from config.toml and from Dupin's own store, each marked with its source. Sent in parts within Dupin's limits on rows and bytes.
- **Actions:** **Copy to Clipboard**, a `copy` action with `template: true`, so Dupin fills `{date}`, `{time}`, `{datetime}` and `{clipboard}` when it is chosen; with `ctx.preferences.paste` (`snippet_paste`) on, **Paste into Front App**, a `paste` template, comes first. A stored snippet also gets **Edit Snippet**, a `push` of the `edit` view (up to 16 KiB of text), and **Delete Snippet**, a `run` with `confirm`. config.toml's are never offered for editing.
- **`views.edit`**: a `form` with name, keyword and text fields, filled in from the snippet being edited (its keyword is the page's `state`).
- **`actions.save`** returns `fieldErrors` for what `snippets.save` refuses, such as a keyword already taken, or `pop` with a toast; **`actions.remove`** calls `snippets.delete`.
- **Capabilities:**
  - `snippets: ["write"]`, for `snippets.save` and `snippets.delete`. It works only from an action you chose or a form you submitted, and can't read snippets or touch config.toml's.
  - `clipboard: ["paste"]`, for the Paste action.

## Files

| Path              | What                                                                      |
| ----------------- | ------------------------------------------------------------------------- |
| `package.json`    | The manifest.                                                             |
| `main.ts`         | `items`, the `edit` form, and the `save` and `remove` actions.            |
| `fixtures/*.json` | Copying, pasting, no snippets yet, and adding, renaming and deleting one. |

## Test it

```sh
dupin ext test extensions/snippets
dupin ext render extensions/snippets extensions/snippets/fixtures/edit.json edit.png
```

Each fixture lists its snippets in `preferences.snippets`, each with a `source` of `config` or `stored`. `edit.json` drives the form with `fill` and `submit` steps, and a change is handed back as the next steps' preferences, as Dupin hands it to the package. Run `dupin` from this checkout's build; see [Working on them](../README.md#working-on-them).
