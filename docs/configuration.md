# Configuration

Dupin reads `$XDG_CONFIG_HOME/dupin/config.toml`, or `~/.config/dupin/config.toml` when `XDG_CONFIG_HOME` is unset. Every key is optional. Settings (⌘, in the panel) and `dupin config set` write changes to a writable `config.toml`, replacing its comments. For a symlink, Dupin writes to the target and keeps the link.

When `config.toml` is read-only, Dupin saves edits in its [encrypted storage](./features/storage.md), one record per top-level key: keys every Mac shares in the `settings` namespace and the rest, such as hotkeys and display choices, in `settings-local`, under `~/Library/Application Support/Dupin/storage` (or `$XDG_DATA_HOME/dupin/storage` when set). Explicit keys in `config.toml` are locked and take precedence. For each remaining top-level key, Dupin uses the saved value if there is one, then the default. If the saved settings can't be read, for example because the storage key in the Keychain is unavailable, Dupin uses the defaults, says why in Settings and saves nothing until they can be read again; `config.toml` itself stays plain TOML. A table such as `[theme]` is one top-level key. If the base later defines a key in the overlay, Dupin removes the shadowed entry on the next successful save and shows a notice. A writable base uses only `config.toml`.

Saves merge the edited keys into the latest valid configuration, preserving unrelated external edits. If the same key changed differently on disk, or an active file is invalid, Dupin reports the problem and leaves both files untouched. An editor that writes between the final source check and the atomic replacement can still have its edit overwritten. Dupin reloads changes while it runs, including atomic replacements and symlink retargets.

Enabling Calendar search or Paste snippets, setting a Selection hotkey for the first time, or turning on an extension such as Window Management in Settings, requests the required macOS access when needed. Startup, opening Settings and edits to `config.toml` only read access status. You can request access before using a feature in Settings → Permissions. See [Permissions](./features/permissions.md).

```toml
display = "mouse" # "mouse", "main", or "active"
position = "center" # "center" puts the field at the middle of the display; "top" near its top
input_source = "ascii" # "ascii", "keep", or an input source identifier such as "com.apple.keylayout.ABC"
```

At launch, a parse error makes Dupin use the defaults and report the problem. During a reload, a parse error or unavailable new hotkey keeps the previous configuration. The error appears in Settings and the next time the panel is shown; a reload never opens the panel itself. Fixing the file clears the error.

In Settings → General → Display, choose Mouse pointer (`"mouse"`, the default), Main display (`"main"`), or Focused window (`"active"`). Focused window uses any existing Accessibility grant. If access or the focused window is unavailable, the panel opens on the main display with a notice. When access is missing, choose **Set Up Accessibility** in the notice to request access or open Accessibility Settings.

## Input source

Like Alfred's "Force Keyboard", Dupin switches the keyboard input source while the panel shows and switches back when it hides, so a Japanese or other input method doesn't hold the query in composition. `"ascii"` leaves a source that already types ASCII alone and otherwise picks the most recently used one that does (ABC, or an input method's Roman mode); an identifier picks that source; `"keep"` leaves the input source alone.

## Nix

The flake's home-manager module installs Dupin and writes `config.toml` from `settings`:

```nix
{
  imports = [ inputs.dupin.homeManagerModules.default ];

  programs.dupin = {
    enable = true;
    settings = {
      hotkey = "alt+space";
      position = "top";
    };
    launchd.enable = true; # start at login
  };
}
```

With `settings` set, `config.toml` links into the Nix store. Settings marks those keys **Managed by Nix** and saves edits to the other keys locally. To change a locked key, edit its Nix value and activate the new generation. To manage it from Settings, remove the key from Nix and activate the new generation.

`extensions` declares extension packages and their grants, and `bundledExtensions` switches the extensions that ship with Dupin on or off and denies their scopes; see [Extensions › Declaring them with Nix](./features/extensions.md#declaring-them-with-nix).

## Feature configuration

- [Advanced composition](./features/advanced.md): `[advanced]`, recomposing sources, filters and kinds
- [Appearance](./features/appearance.md)
- [Calculator](./features/calculator.md#configuration)
- [Calendar](./features/calendar.md#configuration)
- [Clipboard history](./features/clipboard.md#configuration)
- [Dictionary](./features/dictionary.md#configuration)
- [Extensions](./features/extensions.md#turning-them-on)
- [File ranking](./features/files.md#file-ranking)
- [Hotkeys](./features/hotkeys.md#configuration)
- [Keywords and aliases](./features/keywords.md#aliases)
- [Launch at login](./features/settings.md#launch-at-login)
- [Recent items](./features/recent-items.md#configuration)
- [Script commands](./features/scripts.md#configuration)
- [Search selected text](./features/selected-text.md#configuration)
- [Snippets](./features/snippets.md#configuration)
- [System commands](./features/system-commands.md#configuration)
- [Updates](./features/updates.md#configuration)
- [Web searches](./features/web-searches.md#configuration)
- [Window management](./features/window-management.md#configuration)
