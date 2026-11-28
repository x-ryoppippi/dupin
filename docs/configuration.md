# Configuration

Dupin reads `~/.config/dupin/config.toml` (`$XDG_CONFIG_HOME/dupin/config.toml` when `XDG_CONFIG_HOME` is set). Every key is optional. Settings (⌘, in the panel) and `dupin config set` write changes back to it, and Dupin reloads the file while it runs. Settings → General → Advanced → Configuration opens the file or its folder.

```toml
display = "mouse" # "mouse", "main", or "active"
position = "center" # "center", "top", or "bottom"
rows = "auto" # "auto", or a count from 3 to 16
input_source = "ascii" # "ascii", "keep", or an input source identifier
up_key = "" # a chord the lists take as ↑, such as "ctrl+p"; empty for none
down_key = "" # a chord the lists take as ↓, such as "ctrl+n"; empty for none
```

| Key            | Default    | Values                                                                                                                                                          |
| -------------- | ---------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `display`      | `"mouse"`  | `"mouse"` (Mouse pointer), `"main"` (Main display), `"active"` (Focused window). Settings → General → Display                                                   |
| `position`     | `"center"` | `"center"` puts the field at the middle of the display; `"top"` near its top; `"bottom"` near its bottom, with results above it. Settings → General → Position  |
| `rows`         | `"auto"`   | `"auto"` fills the room the display leaves; a count from `3` to `16` shows that many rows before the list scrolls. See [Rows](#rows). Settings → General → Rows |
| `input_source` | `"ascii"`  | `"ascii"`, `"keep"`, or an input source identifier such as `"com.apple.keylayout.ABC"`; see [Input source](#input-source)                                       |
| `up_key`       | `""`       | A chord that works as <kbd>↑</kbd> in the lists, written as `hotkey` is. See [Up and down keys](#up-and-down-keys). Settings → General → Up key                 |
| `down_key`     | `""`       | A chord that works as <kbd>↓</kbd> in the lists. Settings → General → Down key                                                                                  |

With `position = "bottom"` the panel is laid out as fzf and Telescope lay theirs out: the field sits as far above the bottom of the display as `"top"` puts it below the top, and the results run upwards from it, the best match next to the field. The panel's bottom edge stays put as the results change. <kbd>↑</kbd> moves to the row above, the next match, and <kbd>↓</kbd> back towards the field; the ⌘K actions and pickers follow the same order. Hints and notices sit next to the field, and footers, such as the folder picker's <kbd>↩</kbd> Choose folder line, at the top. Forms and cards keep reading top to bottom above the field. The panel doesn't animate its size in this position, so its edge and the field never slide.

### Up and down keys

For a keyboard without arrow keys, `up_key` and `down_key` add a chord that works as <kbd>↑</kbd> and <kbd>↓</kbd> do in the results, the ⌘K actions and pickers, so they follow `position = "bottom"` as the arrows do. The arrows keep working. Settings records the key's position, as it does for `hotkey`, so <kbd>⌃N</kbd> stays on the same key on every layout.

A list key needs <kbd>⌃</kbd> or <kbd>⌥</kbd>, unless it is a function key, and can't be one the panel or its search field already uses: the arrows, <kbd>Home</kbd>, <kbd>End</kbd>, <kbd>Page Up</kbd> and <kbd>Page Down</kbd>, <kbd>↩</kbd>, <kbd>⎋</kbd>, <kbd>⇥</kbd>, <kbd>Space</kbd>, <kbd>⌫</kbd> and <kbd>⌦</kbd> with any modifier, any <kbd>⌘</kbd> chord, <kbd>⌃K</kbd>, which shows a result's actions, or <kbd>⌃A</kbd>, <kbd>⌃E</kbd>, <kbd>⌃B</kbd>, <kbd>⌃F</kbd>, <kbd>⌃D</kbd> and <kbd>⌃H</kbd>, which edit the field. Nor can it be a chord a hotkey, a binding or the other list key has. Settings refuses such a chord and says who has it; in `config.toml` it makes the file invalid. <kbd>⌃P</kbd> and <kbd>⌃N</kbd> suit most keyboards.

Forms and script argument prompts keep the chord for themselves.

Each feature's keys are on its own page; see [Feature configuration](#feature-configuration).

> [!TIP]
> If the file has an error, Dupin keeps working with the settings it can read (at launch) or the previous configuration (on reload), and shows the error in Settings and the next time the panel opens.

<details>
<summary>Errors and reloads</summary>

At launch, a parse error makes Dupin report the problem and use the defaults for what it can't read: each top-level key or table that is unknown or invalid falls back on its own, and the rest of the file still applies, so a key from a newer Dupin doesn't cost your hotkey. The file stays invalid until it's fixed, and Settings saves nothing to it meanwhile. During a reload, a parse error or unavailable new hotkey keeps the previous configuration. The error appears in Settings and the next time the panel is shown; a reload never opens the panel itself. Fixing the file clears the error.

Dupin reloads changes while it runs, including atomic replacements and symlink retargets.

</details>

<details>
<summary>How Settings saves</summary>

Settings and `dupin config set` write changes to a writable `config.toml` in place: a save changes only the keys whose value changed, adds a key to the table that holds it (or a new one at the end), and removes a cleared binding or alias with its own lines. Your comments, key order, blank lines and inline tables stay as you wrote them. For a symlink, Dupin writes to the target and keeps the link.

Saves merge the edited keys into the latest valid configuration, preserving unrelated external edits. If the same key changed differently on disk, or an active file is invalid, Dupin reports the problem and leaves both files untouched. An editor that writes between the final source check and the atomic replacement can still have its edit overwritten.

</details>

<details>
<summary>Read-only config.toml</summary>

When `config.toml` is read-only, Dupin saves edits in its [encrypted storage](./features/storage.md), one record per top-level key: keys every Mac shares in the `settings` namespace and the rest, such as hotkeys and display choices, in `settings-local`, under `~/Library/Application Support/Dupin/storage` (or `$XDG_DATA_HOME/dupin/storage` when set).

- Explicit keys in `config.toml` are locked and take precedence. For each remaining top-level key, Dupin uses the saved value if there is one, then the default.
- A table such as `[theme]` is one top-level key.
- If the saved settings can't be read, for example because the storage key in the Keychain is unavailable, Dupin uses the defaults, says why in Settings and saves nothing until they can be read again; `config.toml` itself stays plain TOML.
- If the base later defines a key in the overlay, Dupin removes the shadowed entry on the next successful save and shows a notice.
- A writable base uses only `config.toml`.

</details>

<details>
<summary>Display and permissions</summary>

Focused window (`display = "active"`) uses any existing Accessibility grant. If access or the focused window is unavailable, the panel opens on the main display with a notice. When access is missing, choose **Set Up Accessibility** in the notice to request access or open Accessibility Settings.

Enabling Calendar search or Paste snippets, setting a Selection hotkey for the first time, or turning on an extension such as Running Apps in Settings, requests the required macOS access when needed. Startup, opening Settings and edits to `config.toml` only read access status. You can request access before using a feature in Settings → Permissions. See [Permissions](./features/permissions.md).

</details>

## Rows

With `rows = "auto"` the result list fills the room the display leaves on its side of the field, so a taller display shows more rows before the list scrolls: at least as many as fit in 416 points (eight rows at the default `row_height`), at most as many as fit in 832. A smaller `row_height` fits more rows into the same room.

A count, such as `rows = 10`, shows that many rows on every display that has room for them. On a display that doesn't, the panel shows as many as fit there, never fewer than three, so the list doesn't run off the screen; the count comes back on a taller display. Each display is measured when the panel opens on it.

Settings → General → Rows recommends a count for the display the panel opens on: three quarters of the room on the list's side of the field, in whole rows of the current `row_height`, from 3 to 16. The rest of the display stays in view around the panel. The recommendation follows `position` and `row_height`, and changes when the panel next opens on another display. **Use N** sets `rows` to it, N being the recommendation. Pick **Auto** from the menu to go back.

## Input source

Like Alfred's "Force Keyboard", Dupin switches the keyboard input source while the panel shows and switches back when it hides, so a Japanese or other input method doesn't hold the query in composition.

| Value         | Effect                                                                                                                                      |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `"ascii"`     | Leaves a source that already types ASCII alone; otherwise picks the most recently used one that does (ABC, or an input method's Roman mode) |
| an identifier | Picks that source                                                                                                                           |
| `"keep"`      | Leaves the input source alone                                                                                                               |

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

Keys set in `settings` show as **Managed by Nix** in Settings; edits to the other keys are saved locally. `extensions` and `bundledExtensions` declare extensions; see [Extensions › Declaring them with Nix](./features/extensions.md#declaring-them-with-nix).

<details>
<summary>Changing a Nix-managed key</summary>

With `settings` set, `config.toml` links into the Nix store. To change a locked key, edit its Nix value and activate the new generation. To manage it from Settings, remove the key from Nix and activate the new generation.

`extensions` declares extension packages and their grants, and `bundledExtensions` switches the extensions that ship with Dupin on or off and denies their scopes.

</details>

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
- [Sync](./features/sync.md#configuration): `[sync]`, the folder this Mac syncs through
- [System commands](./features/system-commands.md#configuration)
- [Telemetry](./features/telemetry.md#configuration): `telemetry`, the anonymous daily usage ping
- [Updates](./features/updates.md#configuration)
- [Web searches](./features/web-searches.md#configuration)
