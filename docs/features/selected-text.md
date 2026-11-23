# Search selected text

Set a **Selection hotkey** in Settings → General. Select text in another app and press the shortcut to open Dupin with that text in the query field. Type to replace the selection. If the app exposes no selected text, Dupin opens with an empty query.

Dupin needs macOS Accessibility access for this shortcut. If access is missing, choose **Open System Settings** in the panel and enable Dupin under **Privacy & Security → Accessibility**, then try again.

## Configuration

Leave `selection_hotkey` empty to keep selected-text search off. Recording the first **Selection hotkey** in Settings → General requests Accessibility access when needed. If you set `selection_hotkey` in `config.toml`, Dupin requests access on the first press that needs it.

```toml
selection_hotkey = "" # optional shortcut for searching selected text
```
