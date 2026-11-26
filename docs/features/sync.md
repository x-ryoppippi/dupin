# Sync

Sync keeps your Macs in step, end-to-end encrypted, through a folder another tool already syncs: iCloud Drive, Dropbox, Syncthing or any synced folder, with no Dupin server or account.

## Open it

Open **Settings → Sync** (<kbd>⌘,</kbd> in the panel), or use `dupin sync` in a terminal.

## What you see

- Before sync is on: the sync folder with **Choose Folder…**, **Turn On Sync**, **Turn On With Passphrase…** and **Join…**.
- Once it is on: the folder, how the key travels, when this Mac last synced, each other Mac with when it last synced, and anything wrong.
- Right after turning it on: the recovery passphrase, shown once, until you press **I've Written It Down**.

**What syncs today:**

- Usage history, the ranking Dupin learns from what you open. Each Mac keeps its own log and every Mac ranks with all of them added together.
- The [snippets](./snippets.md) you add in Dupin, one per keyword: the later edit of a keyword wins, deleting one deletes it on every Mac, and `config.toml`'s snippets stay on each Mac, winning a keyword both hold.
- Settings saved over a read-only `config.toml`, such as one Home Manager writes: the keys every Mac shares (theme, web searches, keyword lists and the like). Device-local ones (hotkeys, display, permissions-related switches, `[sync]`) never leave the Mac, and a writable `config.toml` is still each Mac's own.

Extensions and, if you choose, clipboard history follow.

## What you can do

| Key / action                                                            | What it does                                                                                                                                                                                       |
| ----------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Turn On Sync**, `dupin sync on`                                       | Starts a vault in iCloud Drive's `Dupin` folder when iCloud Drive is set up, with a generated passphrase                                                                                           |
| **Choose Folder…**, `dupin sync on --folder ~/Dropbox/Dupin`            | Uses another folder                                                                                                                                                                                |
| **Turn On With Passphrase…**, `dupin sync on --passphrase-stdin`        | Uses a passphrase of your own (read from stdin on the command line)                                                                                                                                |
| **Join…**, `dupin sync join [--folder …]`                               | Joins another Mac's vault in the same folder; asks for the passphrase in a prompt that never shows it                                                                                              |
| **Sync Now**, `dupin sync now`                                          | Syncs at once. Otherwise a change goes to the folder about two seconds after you make it, another Mac's changes are read as soon as the folder shows them, and a full pass runs every five minutes |
| `dupin sync status`                                                     | Shows the folder, how the key travels, this Mac's device ID, the other Macs with when they last synced, and anything wrong                                                                         |
| **Remove This Mac** on another Mac's row, `dupin sync remove-device ID` | Removes a Mac you no longer use                                                                                                                                                                    |
| **Turn Off**, `dupin sync off`                                          | Stops syncing on this Mac                                                                                                                                                                          |

Add `--json` to `dupin sync` commands for scripts.

## Notes

- The generated passphrase is shown **once**: write it down. You need it to join another Mac unless iCloud Keychain carries the key.
- In **Join…**, leave the passphrase empty to use iCloud Keychain where the build supports it.
- SMB, AFP, WebDAV and FTP shares are refused; see [Which folders](#which-folders).
- A Mac whose clock is more than five minutes ahead has its changes held until the clocks agree.

<details>
<summary>The passphrase and the key</summary>

Without a passphrase of your own, Dupin generates one of six words from the EFF long word list (about 77 bits) and prints it **once**: write it down. A passphrase of your own needs at least 12 characters and can't be a commonly used password. The passphrase protects the vault key in the folder with Argon2id, which takes about half a second once per Mac; it is never stored on disk and never typed on the command line.

A build signed with a keychain access group (not the Nix or ad hoc builds) also carries the key to your other Macs through iCloud Keychain, end-to-end encrypted, so a Mac signed into the same Apple Account joins without the passphrase. The passphrase is written as a backup either way, since Dupin can't tell whether iCloud Keychain is turned on.

Each Mac keeps its own copy of the key in its Keychain (this Mac only, never synchronized), so signing out of iCloud or losing the passphrase never locks data that is already on a Mac.

Everything Dupin puts in the folder is encrypted on your Mac with a key the folder never sees, apart from one small plaintext header, `vault.dupin`, that names the vault and holds the key wrapped under your passphrase. How it works and why is in the [sync design](../design/sync.md).

</details>

<details>
<summary>How usage history syncs</summary>

Each Mac keeps its own log and every Mac ranks with all of them added together, so a Mac you rarely use still benefits from the others. Item paths under your home folder are compared relative to it, so the same file counts as one item on Macs with different user names; a path another Mac recorded is never checked on this one, and only affects ranking when a search here finds that item anyway.

</details>

<details>
<summary>Problems status reports</summary>

- Files that failed to open: they are retried and never deleted.
- A Mac whose clock is more than five minutes ahead: its changes wait until the clocks agree.
- A newer Dupin having written to the folder.

</details>

<details>
<summary>Removing a Mac and turning sync off</summary>

`dupin sync remove-device ID` suits, for example, a Mac that has been away for more than 90 days: the other Macs take over what only it held, then its files leave the folder. If it comes back, it rejoins under a new device ID.

`dupin sync off`: usage history goes back under this Mac's own key at once; its files leave the folder once the other Macs have taken over what only it held, and stay there when no other Mac syncs.

</details>

## Configuration

`[sync]` is never synced; each Mac has its own.

```toml
[sync]
folder = "~/Library/Mobile Documents/com~apple~CloudDocs/Dupin" # wins over the folder chosen with `dupin sync on`
allow_nfs = false
```

| Key         | Default                                | Meaning                                                  |
| ----------- | -------------------------------------- | -------------------------------------------------------- |
| `folder`    | The folder chosen with `dupin sync on` | The synced folder; set here, it wins over the chosen one |
| `allow_nfs` | `false`                                | Allow a folder on an NFS share                           |

With Home Manager, set `programs.dupin.settings.sync.folder`. A folder set this way still needs `dupin sync on` once on the first Mac and `dupin sync join` on the others, because the key can't come from Nix.

## Which folders

Any folder a sync tool keeps in step works, except one inside Dupin's data folder or holding it, and one on an SMB, AFP, WebDAV or FTP share, which can't replace a file in one step. NFS is refused unless `allow_nfs = true` is set in `[sync]`. A folder that already holds a vault is joined, never given a second one.

<details>
<summary>How Dupin uses the folder</summary>

Dupin never follows a link inside the folder, so a link someone plants there can't redirect what Dupin reads or writes. Every Mac writes only its own files, named by random IDs, so sync tools never have a conflict to resolve, and the folder shows neither what syncs nor what anything is called.

</details>

## What it protects

Whoever hosts the folder, or can read it or a backup of it, learns no contents, no names of what syncs and no keys. A Mac that holds the key can read everything; remove a lost Mac with `dupin sync remove-device`.

<details>
<summary>Security details</summary>

Whoever hosts the folder, and anyone who can read it or a backup of it, learns only how many Macs there are and the number, sizes and timing of files. A file changed, truncated, moved or replayed by anyone without the key fails to open and is skipped; an older genuine file put back never replaces a newer one a Mac already holds. Someone with write access can still delete files or stop sync. A lost Mac removed with `dupin sync remove-device` keeps only what it had already synced.

How records are sealed and re-sealed for the folder is in [Encrypted storage](./storage.md) under Format and internals.

</details>

## Acknowledgements

The passphrase word list is the [EFF long word list](https://www.eff.org/deeplinks/2016/07/new-wordlists-random-passphrases) by the Electronic Frontier Foundation, under CC BY 3.0 US. The list of common passwords comes from [SecLists](https://github.com/danielmiessler/SecLists), under the MIT License. Both are in `crates/dupin-sync/data` and listed with their licences in Dupin's third-party notices (`dupin --licenses`).
