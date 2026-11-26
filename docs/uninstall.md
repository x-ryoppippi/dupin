# Uninstalling

Removing the app leaves your configuration and your [encrypted data](./features/storage.md) behind, so installing it again picks up where you left off. Remove those too to leave nothing.

## Remove the app

1. If [sync](./features/sync.md) is on and you are also removing your data, run `dupin sync off`, so your other Macs take over what only this one held.
2. Turn **Launch at login** off in Settings → General (<kbd>⌘,</kbd> in the panel), then press <kbd>⌘Q</kbd> in the panel to quit.
3. Remove the app the way you installed it:

| Installed with          | Remove it with                                                         |
| ----------------------- | ---------------------------------------------------------------------- |
| Homebrew                | `brew uninstall --cask dupin@nightly` (or `dupin`)                     |
| `nix profile install`   | `nix profile remove dupin` (`nix profile list` shows its name)         |
| The home-manager module | Remove `programs.dupin` and switch; its `launchd` agent goes with it   |
| A zip or disk image     | Move `Dupin.app` from `/Applications` (or `~/Applications`) to the Bin |

## Remove your data

> [!WARNING]
> This deletes everything Dupin stores: clipboard history, snippets, snapshots, usage history, extension data and grants, and Settings saved over a read-only `config.toml`. It is encrypted under the Keychain item, so deleting either the item or the folder loses it for good, and a backup of the folder can't be read without the item.

```sh
rm -rf ~/.config/dupin ~/Library/Application\ Support/Dupin ~/Library/Caches/Dupin ~/Library/Logs/Dupin
security delete-generic-password -s app.dupin.Dupin.storage -a data-key
defaults delete app.dupin.Dupin
tccutil reset All app.dupin.Dupin
```

`brew uninstall --zap --cask dupin@nightly` removes the folders and the `defaults` domain, but not the Keychain item or the macOS permissions.

<details>
<summary>What each step removes</summary>

| Where                                 | Holds                                                                                                               |
| ------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| `~/.config/dupin`                     | `config.toml`, [scripts](./features/scripts.md), [extensions](./features/extensions.md) and `extension-grants.toml` |
| `~/Library/Application Support/Dupin` | The encrypted store in `storage/`, and copies a file action left unfinished                                         |
| `~/Library/Caches/Dupin`              | The calculator's exchange rates                                                                                     |
| `~/Library/Logs/Dupin`                | `dupin.log`                                                                                                         |
| Keychain: `app.dupin.Dupin.storage`   | The data key, account `data-key`, named "Dupin storage key"                                                         |
| `defaults` domain `app.dupin.Dupin`   | Whether Dupin has asked for Accessibility                                                                           |
| `tccutil reset`                       | The Accessibility, Calendars and Paste from Other Apps answers macOS keeps for Dupin                                |

With `XDG_CONFIG_HOME`, `XDG_DATA_HOME` or `XDG_CACHE_HOME` set, the configuration is in `$XDG_CONFIG_HOME/dupin`, the store in `$XDG_DATA_HOME/dupin` and the cache in `$XDG_CACHE_HOME/dupin`; remove those as well.

</details>

<details>
<summary>The Keychain items</summary>

`security` reaches the login keychain, where release and Nix builds keep the data key. If it reports that no item was found, search Keychain Access for `app.dupin.Dupin` and delete **Dupin storage key** there.

A Mac that joined sync also holds **Dupin sync key (this Mac)** items, service `app.dupin.Dupin.sync`; delete them the same way, once per item:

```sh
security delete-generic-password -s app.dupin.Dupin.sync
```

A **Dupin sync key** item without "(this Mac)" is in iCloud Keychain and shared with your other Macs. Leave it, and the synced folder (such as `~/Library/Mobile Documents/com~apple~CloudDocs/Dupin`), while another Mac still syncs.

</details>
