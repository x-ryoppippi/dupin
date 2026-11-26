# Sync

Dupin can keep your Macs in step through a folder that some other tool already syncs: iCloud Drive, Dropbox, Syncthing or any synced folder. There is no Dupin server or account. Everything Dupin puts in the folder is encrypted on your Mac with a key the folder never sees, apart from one small plaintext header, `vault.dupin`, that names the vault and holds the key wrapped under your passphrase. How it works and why is in the [sync design](../design/sync.md).

**What syncs today:** usage history, the ranking Dupin learns from what you open. Each Mac keeps its own log and every Mac ranks with all of them added together, so a Mac you rarely use still benefits from the others. Item paths under your home folder are compared relative to it, so the same file counts as one item on Macs with different user names; a path another Mac recorded is never checked on this one, and only affects ranking when a search here finds that item anyway. Settings, snippets, extensions and, if you choose, clipboard history follow.

## Turning it on

In **Settings → Sync**, Turn On Sync uses iCloud Drive's `Dupin` folder when iCloud Drive is set up, or the folder you pick with **Choose Folder…**. Dupin makes a passphrase and shows it once on that page until you press **I've Written It Down**. On another Mac, pick the same folder and press **Join…**: a secure prompt, whose text is never shown, asks for the passphrase. Leave it empty to use iCloud Keychain where the build supports it. The page then shows the folder, how the key travels, when this Mac last synced with **Sync Now**, each other Mac with when it last synced and **Remove This Mac**, anything wrong, and **Turn Off**.

The same is available from the command line:

```sh
dupin sync on                          # in iCloud Drive's Dupin folder, with a generated passphrase
dupin sync on --folder ~/Dropbox/Dupin # in any other folder
dupin sync on --passphrase-stdin       # with a passphrase of your own, read from stdin
```

Without a passphrase, Dupin generates one of six words from the EFF long word list (about 77 bits) and prints it **once**: write it down. A passphrase of your own needs at least 12 characters and can't be a commonly used password. The passphrase protects the vault key in the folder with Argon2id, which takes about half a second once per Mac; it is never stored on disk and never typed on the command line.

On each other Mac, join the same folder:

```sh
dupin sync join                          # asks for the passphrase without echoing it
dupin sync join --folder ~/Dropbox/Dupin
```

A build signed with a keychain access group (not the Nix or ad hoc builds) also carries the key to your other Macs through iCloud Keychain, end-to-end encrypted, so a Mac signed into the same Apple Account joins without the passphrase. The passphrase is written as a backup either way, since Dupin can't tell whether iCloud Keychain is turned on.

Each Mac keeps its own copy of the key in its Keychain (this Mac only, never synchronized), so signing out of iCloud or losing the passphrase never locks data that is already on a Mac.

## Status and other Macs

`dupin sync status` shows the folder, how the key travels, this Mac's device ID, the other Macs with when they last synced, and anything wrong: files that failed to open (they are retried and never deleted), a Mac whose clock is more than five minutes ahead (its changes wait until the clocks agree), or a newer Dupin having written to the folder. `dupin sync now` syncs at once; otherwise Dupin syncs every five minutes. Add `--json` for scripts.

`dupin sync remove-device ID` removes a Mac you no longer use, for example one that has been away for more than 90 days: the other Macs take over what only it held, then its files leave the folder. If it comes back, it rejoins under a new device ID.

`dupin sync off` stops syncing on this Mac. Usage history goes back under this Mac's own key at once; its files leave the folder once the other Macs have taken over what only it held, and stay there when no other Mac syncs.

## Which folders

The folder is refused when it is inside Dupin's data folder, or holds it, and when it is on an SMB, AFP, WebDAV or FTP share, which can't replace a file in one step. NFS is refused unless `allow_nfs = true` is set in `[sync]`. A folder that already holds a vault is joined, never given a second one.

Dupin never follows a link inside the folder, so a link someone plants there can't redirect what Dupin reads or writes. Every Mac writes only its own files, named by random IDs, so sync tools never have a conflict to resolve, and the folder shows neither what syncs nor what anything is called.

## Configuration

`[sync]` is never synced; each Mac has its own.

```toml
[sync]
folder = "~/Library/Mobile Documents/com~apple~CloudDocs/Dupin" # wins over the folder chosen with `dupin sync on`
allow_nfs = false
```

With Home Manager, set `programs.dupin.settings.sync.folder`. A folder set this way still needs `dupin sync on` once on the first Mac and `dupin sync join` on the others, because the key can't come from Nix.

## What it protects

Whoever hosts the folder, and anyone who can read it or a backup of it, learns no contents, no names of what syncs and no keys, only how many Macs there are and the number, sizes and timing of files. A file changed, truncated, moved or replayed by anyone without the key fails to open and is skipped; an older genuine file put back never replaces a newer one a Mac already holds. Someone with write access can still delete files or stop sync. A Mac that holds the key can read everything; remove a lost Mac with `dupin sync remove-device`, and it keeps only what it had already synced.

## Acknowledgements

The passphrase word list is the [EFF long word list](https://www.eff.org/deeplinks/2016/07/new-wordlists-random-passphrases) by the Electronic Frontier Foundation, under CC BY 3.0 US. The list of common passwords comes from [SecLists](https://github.com/danielmiessler/SecLists), under the MIT License. Both are in `crates/dupin-sync/data` and listed with their licences in Dupin's third-party notices (`dupin --licenses`).
