# Settings

Search for a macOS settings pane, such as `bluetooth`, `displays` or `keyboard`, alongside apps. Enter opens the pane in System Settings. Dupin discovers panes that the installed macOS declares as settings extensions and explicitly allows opening through its settings URL scheme; availability and names follow that macOS version. Searching does not open or change settings.

Type `dupin settings` to open Dupin's own preferences. This uses the same window handoff as ⌘,. Settings → Keywords lists every keyword, records the [shortcut](./keywords.md#shortcuts) that opens each command, and edits your [aliases](./keywords.md#aliases) and the Dictionary and snippet keywords. Settings → Snippets lists every [snippet](./snippets.md#adding-and-editing), adds, edits and deletes the ones stored in Dupin, and shows `config.toml`'s read-only.

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/settings/general-dark.png" />
<img alt="Settings → General: hotkey, selection hotkey, display, position, input source, launch at login and more, with the pages in the sidebar." src="../images/settings/general-light.png" width="760" />
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/settings/search-dark.png" />
<img alt="Settings → Search: switches for the calculator, currency conversion, recent items, calendar search, clipboard history and snippet pasting." src="../images/settings/search-light.png" width="760" />
</picture>

Settings saves to `config.toml` when it is writable. With a read-only file, the keys it sets are locked: rows show **Managed by Nix** when the file links into the Nix store, and **Set in config.toml** otherwise. Other edits are saved in Dupin's [encrypted storage](./storage.md). See [Configuration](../configuration.md) for paths and precedence.

**Open config.toml** at the foot of the Settings sidebar opens the file in its default editor, and **Open Config Folder** shows the folder that holds it, with `extensions/` and `shaders/`, in Finder, creating it first if it doesn't exist. **Acknowledgements** opens the licences and copyright notices of the open-source code and data Dupin includes, which Dupin.app carries as `Contents/Resources/ThirdPartyNotices.txt`; `dupin --licenses` prints the same text.

## First run

The first launch opens Settings with a short welcome: choose the hotkey, turn on launch at login, and see which keys switch modes. ⌘Space is Spotlight's shortcut too; to give it to Dupin, turn off Show Spotlight search in System Settings → Keyboard → Keyboard Shortcuts → Spotlight, or choose another hotkey. When another app holds the chord you press, Dupin names it.

The welcome appears once. It is skipped for anyone who used Dupin before: when its usage history exists in [encrypted storage](./storage.md#built-in-namespaces), or an older `history.json` in `~/Library/Application Support/Dupin` that Dupin has yet to move there. Its last step mentions [Settings → Sync](./sync.md) without asking anything about it. **Show Welcome** in the Settings sidebar brings it back.

## Launch at login

In Settings → General, turn **Launch at login** on or off. If macOS requires approval, choose **Open Login Items** and allow Dupin in System Settings. Dupin refreshes the status when you return. This setting is managed by macOS and is available in the signed app bundle outside the Nix store.

Choose **Relaunch** in Settings → General to restart Dupin. If an update is already staged, relaunch installs it first.
