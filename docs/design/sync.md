# Syncing Dupin between Macs

## Problem and goals

The owner uses Dupin on more than one Mac and wants the same snippets, settings, extensions, ranking and, if they choose, clipboard history on each. Today every Mac is an island: `settings.toml`, `history.json` and `extensions.lock` live in the local data directory, and copying them by hand overwrites whatever the other Mac changed.

Sync must work without a Dupin server or account, keep the synced copy unreadable to whoever hosts it, survive the quirks of consumer sync tools, and fit the extension platform so that a built-in feature or an extension gets sync from the host instead of building its own. It builds on [encrypted storage](https://github.com/x-ryoppippi/dupin/pull/230) (`dupin-storage`, documented in `docs/features/storage.md`), the [settings layering](../configuration.md) and the [extension lock file](https://github.com/x-ryoppippi/dupin/pull/229). [Extension platform](./extension-platform.md) · [Tracking issue](https://github.com/x-ryoppippi/dupin/issues/218)

## Decision summary

- **Folder-based sync, no server.** Dupin reads and writes a user-chosen folder that some other tool keeps in step: iCloud Drive by default, Dropbox, Syncthing or any folder sync. There is no Dupin server and no CloudKit: CloudKit needs an iCloud container entitlement that ad hoc and Nix builds cannot carry, and it does not exist on Linux. Linux uses the same folder mechanism.
- **End-to-end encrypted with one vault key.** A random 256-bit vault key seals everything that syncs. It travels as a synchronizable iCloud Keychain item, or, without iCloud Keychain, is wrapped under an Argon2id key derived from a passphrase. Only ciphertext and a small plaintext vault header ever land in the folder.
- **Every device writes only its own files.** Records live under a per-device directory, blobs are content-addressed with deterministic encryption so identical blobs are identical files, and deletions are tombstones. No two devices ever write the same path except identical blobs, so sync tools never have a real conflict to resolve.
- **Last writer wins per record, ordered by a hybrid logical clock.** Usage history is the exception: each device keeps its own log and readers merge them.
- **Sync is a host service on `dupin-storage` namespaces.** A namespace declares a sync mode. Built-ins (settings, snippets, usage, clipboard history, extensions) use it directly; a third-party extension's `storage` namespace syncs only after consent on each Mac.
- **The local store stays the source of truth on each Mac.** The synced folder is a mirror that Dupin exports to and imports from. Searching never waits on the folder, and an evicted or offline folder only delays sync.
- **`dupin-storage` changes before #230 ships data.** Record identity, version stamps, tombstones, blob nonces, key derivation and the key check change now, listed under [Required changes to `dupin-storage`](#required-changes-to-dupin-storage), so turning sync on later is never a format migration.

## Where the folder lives

Settings → Sync → **Turn On Sync** offers iCloud Drive (`~/Library/Mobile Documents/com~apple~CloudDocs/Dupin`) when it exists, and **Choose Folder…** for any other location. On Linux there is no default; the user picks a Dropbox, Syncthing or other synced folder. The chosen path is stored in `[sync] folder`, which is device-local.

Dupin refuses a folder inside its own data directory, inside `storage/`, on a network file system without atomic rename (SMB and AFP shares are refused; NFS is refused unless the user overrides it) or one that another vault's `vault.dupin` already claims with a key this Mac cannot open.

## Keys

### The vault key

Turning on sync on the first Mac creates a vault: a random 256-bit vault key and a random 128-bit vault ID. The vault key is separate from the existing device data key, which stays `this device only` and keeps sealing namespaces that never sync. A synced namespace is sealed under keys derived from the vault key, both locally and in the folder, so exporting a record is a byte copy, not a re-encryption.

| Mode                     | Where the vault key lives                                                                                                                                                                                                                                                                                                                                                                           | Used when                                                                                                                                                                                                                                          |
| ------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| iCloud Keychain          | A generic password item in the data protection keychain, service `app.dupin.Dupin.sync`, account `vault-<vault id>`, `kSecAttrSynchronizable = true`, accessible after first unlock (not `ThisDeviceOnly`). iCloud Keychain carries it end-to-end encrypted to the owner's other Macs.                                                                                                              | The build has a `keychain-access-groups` entitlement and iCloud Keychain is on. Builds without a provisioning profile (ad hoc, Nix, Developer ID without one) fall back to the login keychain, which cannot synchronize, so they use a passphrase. |
| Passphrase               | `vault.dupin` holds the vault key wrapped with XChaCha20-Poly1305 under a key-encryption key from Argon2id. Each Mac asks for the passphrase once when it joins, then keeps the unwrapped vault key in its own keychain as `vault-<vault id>`, `ThisDeviceOnly` and not synchronizable. On Linux it keeps it in `vault.key`, a `0600` file beside `storage.key`, under the same rules as `FileKey`. | No iCloud Keychain, Linux, or the user prefers it.                                                                                                                                                                                                 |
| iCloud Keychain + backup | Both: the synchronizable item and a passphrase-wrapped copy in `vault.dupin`. Settings recommends this, because losing every Mac or turning off iCloud Keychain otherwise loses the vault.                                                                                                                                                                                                          | The user sets a recovery passphrase while using iCloud Keychain.                                                                                                                                                                                   |

Wrapping a random vault key, instead of deriving the vault key from the passphrase, lets the passphrase change without re-sealing anything.

**KDF: Argon2id (RFC 9106), m = 256 MiB, t = 3, p = 4, 16-byte random salt, 32-byte output.** This takes about half a second on an Apple silicon Mac and runs once per Mac, at join or passphrase change, so the cost buys offline-guessing resistance without slowing daily use. The parameters and salt are stored in `vault.dupin`, so a later version can raise them by rewrapping. Dupin refuses parameters below m = 64 MiB, t = 3 when joining, so a tampered header cannot downgrade the KDF. Settings proposes a generated six-word passphrase from the EFF long word list (about 77 bits) and accepts a user-chosen one of at least 12 characters that is not in a bundled list of common passwords.

### Device identity

Each Mac has a random 128-bit device ID, stored in its keychain as `device-id`, `ThisDeviceOnly` (on Linux, in the data directory with a hash of `/etc/machine-id`). `ThisDeviceOnly` items are not restored to a different Mac by Migration Assistant or a backup, so a cloned or restored Mac mints a new ID instead of writing as the old one. A Linux machine whose `machine-id` hash no longer matches mints a new ID too. The device ID is never a secret; it names directories and orders ties.

### What derives from the vault key

HKDF-SHA256 with salt `dupin-storage v2` and info that names the purpose, the vault ID, the key epoch and the namespace:

- the namespace seal key and blob MAC key (as #230 already derives from the data key);
- a blob nonce key, for deterministic blob nonces;
- a name key, which turns a namespace name into its folder ID and a logical record key into its key ID, so neither appears in the folder.

The **key epoch** starts at 1 and increases when the user rotates the vault key.

## Folder layout

```
<folder>/
  vault.dupin                          plaintext header: format, vault ID, epoch, key mode, KDF parameters and salt,
                                       wrapped vault key (passphrase modes), and a value sealed under the vault key
  devices/<device id>.dev              sealed descriptor, written only by that device
  ns/<namespace id>/
    <device id>/<key id>.rec           that device's latest version of one record, or its tombstone
    <device id>/0000…0000.rec          that device's clear marker for the namespace (reserved key ID)
    blobs/<blob id>.blob               content-addressed blob, deterministic ciphertext
  tmp.nosync/<device id>/              staging for writes that cannot be staged locally
```

Every ID is 32 lowercase hexadecimal characters, so names are the same on case-insensitive and case-sensitive file systems and no sync tool rewrites them. A name that does not match this grammar exactly is ignored; that is how Dupin passes over sync-tool artefacts.

**The device descriptor** is a sealed JSON record: display name ("Work MacBook Pro"), platform, Dupin version, format version, the namespaces the device takes part in, `seen_at` (the device's hybrid logical clock when it last finished a full import) and `applied` (per namespace, the highest version it has imported from each other device). A device rewrites it after each full import, at most once a minute. Because it is sealed under the vault key, it is also that device's key check: a descriptor that opens proves the device and this Mac hold the same key.

**Records.** A device keeps one file per logical record in its own directory: the newest version it wrote of that record. When it writes a newer version, it replaces its own file. Readers gather every device's file for a key and take the newest version. A device removes its own file for a key once the file is superseded by a newer version from another device that it has imported, so each key converges on one file plus a tombstone at most.

**Blobs** are named by the keyed hash of their plaintext, as in #230, and sealed with a nonce derived from the blob nonce key and that hash. Two devices that store the same image therefore write byte-identical files at the same path. A sync tool sees the same content and keeps it; a tool that still makes a conflicted copy produces two valid files with the same plaintext, and the copy is ignored and later removed. Deterministic encryption leaks only whether two blobs are equal, which the content-addressed name already reveals.

## Versions and conflicts

### Hybrid logical clock

A version stamp is `(hlc, device id)`. `hlc` is 64 bits: the upper 48 bits are Unix milliseconds and the lower 16 a counter. Each Mac keeps one clock across all namespaces, persisted in its local store.

- **Stamping a local write:** `hlc = max(last + 1, now_ms << 16)`. A counter overflow carries into the milliseconds.
- **Importing a version:** `last = max(last, imported hlc)`. A device that has seen another's write stamps its next write later, even when its own wall clock is behind.
- Stamps compare as integers, with the device ID breaking ties, so every device picks the same winner.

When an imported stamp is more than five minutes ahead of the local wall clock, Settings → Sync names the device whose clock is ahead. The version is still accepted: the clock keeps causality, so a later edit anywhere still wins; refusing the version would only split the devices.

### Why last writer wins

Each synced record is a whole value: one setting, one snippet, one clipboard entry, one extension pin. Concurrent edits of the same record on two Macs within one sync interval are rare, and when they happen the user expects the later one. Field-level CRDTs would add a merge format per owner for no visible gain. Usage history is different: its value is counts and visit times that every device adds to, so taking one device's copy would lose the others' visits. It uses per-device logs and a merge instead (see [Merge modes](#merge-modes)).

### Merge modes

A namespace declares one of:

- **`off`** (the default): never leaves the Mac, sealed under the device data key.
- **`lww`**: records keyed by a logical key; the newest version across devices wins.
- **`per_device`**: each device writes only records under its own key, and the owner reads every device's records and merges them. The host never chooses between them.

### Deletions and tombstones

Deleting a record in a synced namespace writes a tombstone version: the record's header with `deleted = true` and no body. A tombstone wins like any newer version, so a stale copy arriving later loses to it.

A device drops a tombstone from its own directory only when all of these hold:

1. it is at least 30 days old;
2. every device that takes part in the namespace and is not retired reports, in its descriptor, an `applied` stamp from this device at or above the tombstone's stamp, so each has seen the deletion;
3. no device directory still holds an older version of the key.

Until then the tombstone stays, so a Mac that comes back after months cannot resurrect the record. A device unseen for 90 days blocks this, so Settings → Sync lists it as inactive with **Remove This Mac**. Removing a device writes a retirement entry into every remaining device's descriptor; once all of them carry it, each live device ignores the retired device's directory and the device with the lowest ID deletes it.

**Clear History** writes a clear marker, the reserved key ID `0000…0000`, holding `clear_before = <stamp>`. Every version older than the newest marker across devices is deleted, without a tombstone per record. A copy made after the clear survives it.

### Retention and quotas with several devices

Limits are a pure function of the merged view, so every device evicts the same records and none needs to tell the others:

- **Age** is measured from the physical part of the winning version's stamp. An imported record already past the limit is never shown.
- **Entry and byte limits** order the merged records by `(stamp, key id)` and evict from the oldest. Import applies limits before records become visible, so an evicted record that another device has not yet swept never reappears.
- Retention never writes tombstones. Each device deletes only its own superseded and evicted files. Files in another device's directory are ignored until that device sweeps them or is retired.
- The limits of a synced namespace are a synced record of the namespace, so every device evicts against the same numbers. For clipboard history, `max_entries` and `max_entry_bytes` move into that record when clipboard sync is on.

**Blobs** have no owner, so any device may remove one. A blob is removed only after this device has observed it unreferenced by every device's records for 7 days in a row. If a removal races with a record that still needs the blob, the device that wrote the record notices the blob missing from the folder and uploads it again from its local copy. Any device that holds a blob locally and sees a live record refer to it re-uploads a missing blob.

### Partial sync

Sync tools deliver files in any order and at any time. Dupin tolerates every interleaving:

- **A record whose blob has not arrived** is imported but hidden, and shown once the blob arrives. After 24 hours it appears with a "Waiting for <device>" placeholder that cannot be copied. The open-time rule in #230 that deletes a record with a missing blob does not apply to synced namespaces.
- **A file that fails to open** (truncated, mid-write, a placeholder, damaged) is skipped and retried on the next scan. Dupin never deletes a file in the folder because it fails to open; only its writer removes its own files.
- **A descriptor or vault header that has not arrived** delays joining, never guesses.

### Sync-tool artefacts

| Artefact                                                                                                                               | Handling                                                                                                                                                                                                                                                                                                      |
| -------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Conflicted copies (`name 2.rec` from iCloud, `name (… conflicted copy …).rec` from Dropbox, `name.sync-conflict-….rec` from Syncthing) | Do not match the name grammar, so they are ignored. Dupin deletes them only in its own device directory and in `blobs/`, once the canonical file opens. In its own directory a conflicted copy means two machines write as one device ID, so Dupin also mints a new ID (see [Failure modes](#failure-modes)). |
| iCloud placeholders: `.name.icloud` stubs, and dataless files under File Provider (macOS 14 and later)                                 | A stub counts as not arrived. Before reading, Dupin checks `SF_DATALESS` with `stat`, never reads a dataless file on a worker that would block, and asks for the download (`startDownloadingUbiquitousItem(at:)`). Settings suggests **Keep Downloaded** for the folder in Finder.                            |
| Partially written files, and sync tools' own temporary names (`.syncthing.*.tmp`, `~syncthing~*`, `.dropbox*`)                         | A temporary name does not match the grammar. A partial file under a real name fails authentication and is retried.                                                                                                                                                                                            |
| Files restored by "version history" or undelete                                                                                        | They are older versions or resurrected tombstoned files. The newest stamp still wins, and a device never re-adopts a version older than the one it holds.                                                                                                                                                     |
| `.DS_Store` and other unrelated files                                                                                                  | Ignored.                                                                                                                                                                                                                                                                                                      |

**Atomic writes.** Dupin stages a write in its local `storage/tmp/` when that is on the same volume as the folder (it compares `st_dev`) and renames it into the folder; otherwise it stages in `tmp.nosync/<device id>/` inside the folder. iCloud Drive does not upload names ending `.nosync`, and the other tools may upload a partial temporary file, which no reader looks at. After the rename Dupin flushes the directory, as #230 does. A device clears its own `tmp.nosync` directory on start.

**vault.dupin races.** It is created with an exclusive create. When two Macs create a vault at about the same time, a sync tool keeps both as `vault.dupin` and a conflicted copy. Each Mac rereads the folder a minute after creating, and the vault with the lower vault ID wins. The other Mac joins it and re-seals its synced namespaces under the winning key; until then it does not export.

## When sync runs

The sync engine runs in the app process on its own worker, never on the UI thread and never in the extension-host helper. It exports a namespace's changed records two seconds after its last write and imports when a file system event (FSEvents, inotify) arrives under the folder, plus a full scan every five minutes and on wake. A full import lists every device directory, opens only the headers of files whose size or modification time changed since the last scan, and applies winners into the local store. A scan never blocks a search.

## What syncs

| Data              | Namespace / mode          | Decision                                                                                                                                                                                                                              |
| ----------------- | ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Settings          | `settings`, `lww`         | One record per shared top-level key of `settings.toml` (a table such as `[theme]` is one key, as in the layering today). `config.toml` never syncs. See [Settings](#settings).                                                        |
| Snippets          | `snippets`, `lww`         | One record per snippet, keyed by its keyword. See [Snippets](#snippets).                                                                                                                                                              |
| Extensions        | `extensions`, `lww`       | One record per Git-installed package from the lock: `source`, `ref`, `commit`, `content_hash`, `version`, `keyword`, `requested`. Never `granted` or `pending_consent`. See [Extensions](#extensions).                                |
| Usage history     | `usage`, `per_device`     | Each device writes one record: its own frecency entries. Readers merge. See [Usage history](#usage-history).                                                                                                                          |
| Clipboard history | `clipboard`, `lww`        | Opt-in and off by default, separately from clipboard history itself. See [Clipboard history](#clipboard-history).                                                                                                                     |
| Extension storage | `ext.<package>`, declared | Only with the package's `storage.sync` scope, granted on each Mac. See [How it plugs into the platform](#how-it-plugs-into-the-platform).                                                                                             |
| Never             | —                         | macOS permissions and TCC decisions, extension grants, device-specific hotkeys, display choices, the sync settings themselves, keychain items other than the vault key, snapshots, logs, caches and the extensions folder's contents. |

Each synced namespace has a device-local switch in Settings → Sync: settings, snippets, extensions and usage history are on when sync is turned on; clipboard history is off. A Mac with a namespace off neither exports nor imports it and is not counted as taking part in it for tombstone collection.

### Settings

Each configuration key is classified in the feature manifest as **shared** or **device-local**. A key with no classification is device-local, so a new key never syncs by accident. Device-local keys:

- hotkeys: `hotkey`, `selection_hotkey`, `[bindings]`;
- display choices: `display`, `position`, `input_source`, `backdrop` (it can name a local WGSL file);
- every key whose enabling requests macOS access or authority on that Mac: `calendar`, `window_management`, `system_automation`, `snippet_paste`, `extensions`, `[clipboard]`; Settings → Permissions already treats these as per-Mac decisions;
- `auto_update` (Nix-installed Macs must not inherit it), `[sync]`, and paths such as script folders.

Everything else, including `[theme]`, `[web_searches]`, `web_fallbacks`, keyword lists, `calculator`, `calculator_currency`, `show_recent`, `file_limit` and `file_demotions`, is shared.

Precedence is unchanged on each Mac: explicit keys in `config.toml` (the Nix base) are locked and win, then `settings.toml`, then defaults. Synced values land in `settings.toml`, and Dupin writes them there with the existing merge-save, so the file stays the readable local view and an external edit to a shared key is exported as a new version. Two changes to the layering:

- **With sync on, `settings.toml` is always the overlay for shared keys**, even when `config.toml` is writable. Turning on sync offers to move shared keys from a writable `config.toml` into `settings.toml`; a key left in `config.toml` stays locked on that Mac and does not sync.
- **Removing a shadowed overlay entry never writes a tombstone.** Today, when the base defines a key that the overlay also holds, the next save removes the overlay entry. With sync, that removal is local: the synced record stays, so a Mac whose Nix base locks `theme` does not delete the theme on a Mac without that lock.

### Snippets

User snippets move out of the config files into the `snippets` namespace. The record key is the keyword, so the same keyword created on two Macs is one record and the later edit wins. Renaming a keyword deletes the old record and creates the new one. `[snippets]` in a read-only `config.toml` remains a base: it is never exported and wins over a synced snippet with the same keyword on that Mac. Turning on sync offers once to move snippets from a writable `config.toml` or `settings.toml` into the namespace; snippets left there are not synced.

### Extensions

Installing, updating or removing a Git-sourced package with `dupin ext` also writes its record. On another Mac, the engine fetches the locked commit, checks the content hash and places the package exactly as `dupin ext sync` does: nothing from the package runs. The package arrives **with no grants and disabled**: Settings → Extensions lists it as "Installed from <device>" and enabling it asks for every requested scope on this Mac. An update on another Mac moves this Mac to the new commit, keeps grants for scopes still requested, drops removed scopes and puts added scopes in `pending_consent`, as a local update does. A removal elsewhere removes the package and its grants here and shows a notice.

Folder-sourced packages are not synced, because their path means nothing on another Mac. Nix-declared packages are not in the lock and are not synced; a synced package whose name matches a Nix-declared one is ignored on that Mac with a notice. Settings → Sync can turn off **Install extensions added on other Macs**; the records still sync and are listed for a manual install.

### Usage history

`history.json` moves into the `usage` namespace in the local store, so it is also encrypted at rest. Each device writes a single record keyed by its device ID: its own entries (count, latest visits, query counts), including those recorded before sync was turned on, because ranking already depends on them. It rewrites it at most every five minutes and when Dupin quits. The ranked history is the merge of every device's record: counts and query counts add, visits are the union of the latest ten per item, and the 500 most frecent items are kept. Item IDs that are paths under the home folder are stored relative to `~`; an ID that resolves to nothing on this Mac (an app that is not installed) is kept but not shown.

### Clipboard history

Clipboard sync is a separate switch, **Sync clipboard history**, off by default, under Settings → Sync and only available while clipboard history is on. It exports only copies made after it is turned on, as clipboard history itself records only copies made after it is turned on. The concealed, transient, auto-generated and ignored-app rules are applied before recording, so excluded copies are never stored and never synced.

- **Text** syncs. The record key is the keyed hash of the text, so the same text copied on two Macs is one entry and moves to the top.
- **Files sync as references only**: home-relative path, display name, size and modification time, never the contents. On a Mac where the path resolves to a file of the same size, Enter copies that file; otherwise the row says "On <device>" and Enter copies the path as text.
- **Images**, once clipboard history records them, sync only with a second switch, **Include images**, off by default, and only up to 8 MiB each; a larger image stays on the Mac it was copied on. An image is a blob, so the same screenshot copied twice is stored once.
- **Rich text** is out of scope until clipboard history records it; it then follows images.

## How it plugs into the platform

Sync is a host service in a new crate, `dupin-sync`, which depends on `dupin-storage` and has no OS dependencies. Platform adapters supply the vault key provider (Keychain or file), file system events and iCloud download requests. A namespace's owner chooses the sync mode when it opens the namespace; the owner never sees the folder, device directories or stamps. It sees records with an extra read-only field naming the device that wrote the winning version, and, for `per_device`, every device's records.

Built-in features open their namespaces directly. Extensions reach storage only through the broker's `storage` capability (#230), and sync is a separate scope of it:

```json
"capabilities": {
	"storage": { "sync": true }
}
```

- Without `sync`, the namespace is `off`. With it, the lock records `storage:sync` in `requested`, and the namespace syncs only once the user grants that scope on this Mac. Like every grant, it is re-consented on each Mac. Consent says that the extension's stored data will be copied, encrypted, to the sync folder and to the user's other Macs.
- The namespace is named from the package's lock source and package name, not its installation ID, so the same package on two Macs shares one namespace. Folder-sourced and Nix-declared packages have no portable identity and cannot be granted `storage:sync`.
- Revoking the scope stops export and import at once and removes this device's files for that namespace from the folder. Data on other Macs stays until they revoke too or the package is removed.
- An extension's merge mode is `lww`. `per_device` stays for built-ins until an extension needs it.

## Required changes to `dupin-storage`

These change the format in #230. Landing them in #230, or right after it and before any release writes data, means sync never needs a migration: a namespace that starts local and later syncs is re-sealed under the vault key, which is a copy, not a format change.

1. **Format version 2**, and HKDF salt `dupin-storage v2`.
2. **Record identity is a stable 128-bit key ID, separate from its version.** Today a record's ID is `(millis, seq)`, its creation time, and every `put` makes a new record. Add `put(key, …)` as an upsert: the owner supplies a logical key, which becomes a key ID through the namespace's name key, or none, in which case the key ID is random. Local files become `records/<key id>.rec`, holding the winning version.
3. **A sealed version header** in every record: `stamp` (hybrid logical clock and origin device ID), `created_at` (wall milliseconds, kept across updates), `deleted` and the blob list. `list` orders by `(stamp, key id)` and its `before` cursor is that pair. The hybrid logical clock replaces the "time even if the clock went back" rule for ordering; `created_at` still drives age limits for local-only namespaces.
4. **Associated data binds identity, not path**: format version, kind, key source (device key, or vault ID and epoch), namespace ID, origin device ID and key ID for a record; the blob hash for a blob. The same bytes then open in `records/` and in `ns/<namespace id>/<device id>/`. Moving a file to another namespace or another key still fails, as in #230.
5. **Key derivation names its key source.** Namespace keys derive from either the device data key or the vault key, and the info string includes the source, vault ID and epoch. Add the blob nonce key and the name key.
6. **Deterministic blob nonces**: the nonce is the first 24 bytes of `HMAC(blob nonce key, blob hash)`. Records keep fresh random nonces, because only one device ever writes a given record file.
7. **Tombstones.** `delete` writes a tombstone version instead of unlinking. A local-only namespace drops a tombstone as soon as the write is durable, so behaviour without sync is unchanged; a synced namespace keeps it under the rules above. Add the reserved clear marker for **Clear History**.
8. **Key check per device and key source.** `key.check` seals the key source and vault ID, so a namespace sealed under a vault key is not taken for damage when the device key opens it, or the other way round. It stays per Mac and never syncs; the folder's checks are `vault.dupin` and each device's descriptor.
9. **No deletion on doubt in synced namespaces.** A record with a missing blob is kept as pending instead of deleted, and a file that fails to open in an import is skipped, never removed. Local damage handling in a local-only namespace stays as #230 has it.
10. **Namespace IDs.** A namespace's folder name is its name through the name key, so the folder does not reveal which features or extensions sync. Local `storage/<namespace>/` keeps plain names.
11. **A per-namespace sync limit on item size**, at most 64 MiB, below the 512 MiB local ceiling, so one large blob cannot stall a consumer sync tool.

## Threat model

**Assets**: clipboard history (most sensitive), snippets (may hold secrets), usage history and queries (behaviour), settings and the extension list.

**Protected:**

- The sync provider, anyone with read access to the folder, and a backup of the folder learn no contents, no namespace names and no logical keys. Without the vault key they hold ciphertext and the wrapped key in `vault.dupin`.
- Tampering by anyone with write access to the folder is detected: a modified, truncated or moved file fails to open and is skipped.
- A passphrase-wrapped vault key resists offline guessing as far as the passphrase and Argon2id parameters allow.
- With iCloud Keychain, the vault key is protected by iCloud Keychain's own end-to-end encryption, whether or not iCloud Advanced Data Protection is on for iCloud Drive.

**Not protected:**

- **Metadata**: the number of devices, the number and sizes of records and blobs per namespace and device, and when files change. Blob equality is visible.
- **Availability**: whoever controls the folder can delete files, withhold them or stop sync.
- **Rollback**: someone with folder write access can put back an older, genuine version of a file. A Mac never adopts a version older than one it already holds, so this only affects a Mac that has not yet seen the newer version, such as a newly joined one.
- **A device that holds the vault key** can read everything and write records that look as if any device wrote them. There are no per-device signatures; all devices belong to one person, and a compromised Mac has the key anyway.
- **A lost Mac**: removing it stops its directory being read, but it keeps what it already synced. **Rotate Vault Key** in Settings → Sync creates a new epoch: every device re-seals its synced namespaces and re-exports, then deletes its old-epoch files. With iCloud Keychain the lost Mac should also be removed from the Apple Account, and in passphrase mode the passphrase must change too. The lost Mac can still read data from before the rotation.
- Same-user code on any Mac, as for local storage in #230.

## Failure modes

| Failure                                                       | Behaviour                                                                                                                                                                                                         |
| ------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Folder missing, unmounted or offline                          | Local use continues; Settings → Sync shows "Folder unavailable" and the engine retries on events and every five minutes. Nothing is deleted locally.                                                              |
| iCloud evicts files                                           | Treated as not arrived; download requested. Local data is unaffected.                                                                                                                                             |
| Vault key unavailable (iCloud Keychain off or not yet synced) | The Mac does not join and asks for the recovery passphrase if `vault.dupin` has one. Local data stays.                                                                                                            |
| Wrong passphrase                                              | Refused without changing anything. There is no attempt limit, since the attacker who matters works offline.                                                                                                       |
| Vault key lost on every Mac and no passphrase                 | The vault cannot be opened. Each Mac keeps its local copy and can start a new vault; the old folder contents are unreadable and **Start Over** deletes them.                                                      |
| Two vaults created at once                                    | The lower vault ID wins; the other Mac joins and re-seals.                                                                                                                                                        |
| Cloned Mac writing as another device                          | A conflicted copy in its own directory, or a version in its own directory it did not write, makes it mint a new device ID and re-export. The old ID's directory belongs to the other machine.                     |
| Clock far wrong                                               | Ordering stays causal through the clock; Settings names the skewed device. Age limits on that device's records are measured from its stamps, so a far-future clock keeps its entries until real time passes them. |
| Device away longer than 30 days                               | Tombstones are kept for it until it reports them applied, or until it is removed.                                                                                                                                 |
| Folder full or quota exceeded                                 | Writes fail and are retried; a namespace over its sync limit stops exporting and Settings says so.                                                                                                                |
| Unknown newer format in the folder                            | A device with an older Dupin reads only the versions it understands and does not export, and Settings asks to update.                                                                                             |

## Rollout

1. Make the [format changes](#required-changes-to-dupin-storage) in #230 or straight after it, before a release stores data.
2. Move `history.json` into the `usage` namespace and snippets into the `snippets` namespace, still local.
3. Add `dupin-sync` with the folder layout, the hybrid logical clock, import and export, tombstones, retention and artefact handling. Test it with a deterministic simulation: several virtual devices over a fault-injecting folder that reorders, delays, duplicates and truncates files, makes conflicted copies and placeholders and skews clocks, and check that every device converges and nothing deleted comes back.
4. Add the vault: the passphrase mode first, since it works in every build and on Linux, then the synchronizable Keychain item in builds that carry the access group entitlement. Add `dupin sync init|join|status|remove-device|rotate` with `--json`.
5. Sync settings with the shared/device-local classification and the layering changes.
6. Sync snippets and extension records with reinstall per Mac.
7. Sync usage history.
8. Add opt-in clipboard sync for text and file references, then images.
9. Add `storage.sync` for extensions behind its consent.
10. Run the same suite on Linux with Syncthing and Dropbox folders.

## Open questions

- Will the release build carry a `keychain-access-groups` entitlement and provisioning profile, so iCloud Keychain mode exists outside development, or is passphrase mode the norm?
- Are 30 days for tombstones, 90 days for inactive devices and 7 days for unreferenced blobs right for how often the owner's Macs are offline?
- How does iCloud Drive cope with thousands of small files per namespace? If it is slow, pack each device's records into per-device segment files, keeping the one-writer rule.
- Should Nix-declared extensions get a portable identity (an optional `source` in the module), so they can sync storage?
- Should script commands and their folder sync? They run with user authority, so syncing them is a separate consent question.
- Windows: the same folder design with DPAPI or Windows Hello for the local copy of the vault key.
