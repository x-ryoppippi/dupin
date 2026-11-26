# Search selected text

A hotkey opens Dupin with the text selected in another app already in the query field.

## Open it

Select text in another app and press your **Selection hotkey**.

## What you see

- The panel, with the selected text as the query.
- An empty query if the app exposes no selected text.

## What you can do

| Key / action | What it does                           |
| ------------ | -------------------------------------- |
| Type         | Replace the selection with a new query |

Other keys: [Keys](../usage.md#keys).

## Notes

- Off until you set a **Selection hotkey**.
- Needs macOS Accessibility access. Recording the first **Selection hotkey** in Settings → General requests it when needed; if you set `selection_hotkey` in `config.toml`, Dupin requests it on the first press that needs it.
- If access is missing, choose **Open System Settings** in the panel and enable Dupin under **Privacy & Security → Accessibility**, then try again.

## Configuration

```toml
selection_hotkey = "" # optional shortcut for searching selected text
```

| Key                | Default | Meaning                                                                                       |
| ------------------ | ------- | --------------------------------------------------------------------------------------------- |
| `selection_hotkey` | `""`    | The shortcut; empty keeps selected-text search off. Settings → General → **Selection hotkey** |

[Hotkeys](./hotkeys.md) · [Permissions](./permissions.md)
