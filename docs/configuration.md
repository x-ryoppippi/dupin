# Configuration

Settings (⌘, in the panel) changes the hotkeys, where the panel opens, the input source, how many files a search shows, whether clipboard history is kept, whether Enter pastes snippets and the backdrop. Changes apply at once and are written to `config.toml`, replacing any comments in it. A symlinked `config.toml` is written where it points, so the link stays; a read-only one, such as one [Nix manages](#nix), is shown in Settings but not changed there.

Saving from Settings merges the changed fields into the latest valid file, preserving unrelated external edits observed before saving. If the same field changed differently in the file, or the file is invalid, Settings reports the problem and leaves the file untouched. The final source check is best-effort: an editor that writes between that check and the atomic replacement can still have its edit overwritten.

Dupin reads `$XDG_CONFIG_HOME/dupin/config.toml`, or `~/.config/dupin/config.toml` when `XDG_CONFIG_HOME` is unset. Every key is optional; a missing file means the defaults. File changes apply automatically while Dupin runs, including atomic replacements and changes to a linked file. The panel and Settings use the same configuration.

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

With `settings` set, `config.toml` is a read-only link into the Nix store. Change it in Nix and activate the new generation; Dupin follows the updated link automatically. Leave `settings` empty to keep the file out of Nix and editable from Settings.

## Feature configuration

- [Appearance](./features/appearance.md)
- [Calculator](./features/calculator.md#configuration)
- [Calendar](./features/calendar.md#configuration)
- [Clipboard history](./features/clipboard.md#configuration)
- [Dictionary](./features/dictionary.md#configuration)
- [Extensions](./features/extensions.md#turning-them-on)
- [File ranking](./features/files.md#file-ranking)
- [Hotkeys](./features/hotkeys.md#configuration)
- [Launch at login](./features/settings.md#launch-at-login)
- [Recent items](./features/recent-items.md#configuration)
- [Script commands](./features/scripts.md#configuration)
- [Search selected text](./features/selected-text.md#configuration)
- [Snippets](./features/snippets.md#configuration)
- [System commands](./features/system-commands.md#configuration)
- [Updates](./features/updates.md#configuration)
- [Web searches](./features/web-searches.md#configuration)
- [Window management](./features/window-management.md#configuration)
