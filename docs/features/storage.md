# Encrypted storage

`dupin-storage` is the host's store for data that must outlive a restart but should not sit on disk in the clear, such as clipboard history. Each owner opens its own namespace: a native feature by name, and an extension, such as the bundled clipboard history, through the broker's `storage` capability, which maps the installed package to a namespace of its own (see [Extension namespaces](#extension-namespaces)). The owner never sees paths or keys; it stores JSON records and the blobs they refer to, and reads them back.

## What Dupin keeps where

What Dupin learns about its user belongs in encrypted storage, in the namespace the [sync design](../design/sync.md#what-syncs) assigns it; the table lists every file that holds such data and where each stands. A built-in store that can't be opened, because the key is missing or the data was sealed with another, keeps what it holds untouched, saves nothing until it opens and says why on every Settings page; it never falls back to a file in the clear.

| What                                   | Where                                                                              | Form                                                   | Syncs                                                    |
| -------------------------------------- | ---------------------------------------------------------------------------------- | ------------------------------------------------------ | -------------------------------------------------------- |
| Snippets added in Dupin                | `snippets` namespace, one record per snippet                                       | Encrypted                                              | `snippets`, last writer wins, when it ships              |
| Saved snapshots                        | `snapshots` namespace, one record per snapshot                                     | Encrypted                                              | Never                                                    |
| Clipboard history                      | The bundled clipboard history package's namespace                                  | Encrypted                                              | Opt-in, when clipboard sync ships                        |
| An extension's stored data             | `ext-u-…` or `ext-b-…`, its own namespace                                          | Encrypted                                              | Only with `storage.sync`, when it ships                  |
| Usage history (picks and queries)      | `history.json` in `~/Library/Application Support/Dupin`, until it moves to `usage` | Plain JSON, `0600`                                     | `usage`, per device, when it moves                       |
| Settings edits over a read-only base   | `settings.toml` beside `storage/`, until it moves to `settings`                    | Plain TOML                                             | `settings`, shared keys only, when it moves              |
| Extension grants and switches          | `grants.toml` beside `storage/`, until it moves into storage                       | Plain TOML                                             | Never                                                    |
| Installed packages                     | `extensions.lock` beside `storage/`, until it moves to `extensions`                | Plain TOML                                             | `extensions`, Git-installed packages only, when it moves |
| `config.toml`, `extension-grants.toml` | Dupin's config folder                                                              | Plain TOML: the user's own files, often written by Nix | Never                                                    |
| The data key                           | The Keychain (see [The data key](#the-data-key))                                   | Keychain item                                          | Never                                                    |
| `storage.state`                        | Beside `storage/`                                                                  | Sealed under the data key                              | Never                                                    |

Dupin also writes, in the clear, files that hold no user data:

- `~/Library/Logs/Dupin/dupin.log`: startup problems, updates and failures by kind. It never holds what you typed, a result's title or path, clipboard contents or snippet text; an extension's failure is logged by its code and line only, since its message is the extension's own text.
- `~/Library/Caches/Dupin/calculator` (or `$XDG_CACHE_HOME/dupin/calculator`): the European Central Bank's published exchange rates and when they were last fetched.
- `welcomed`, an empty marker that the welcome was shown, and the example shaders Settings copies into the config folder.
- `copy-staging/`, while a copy runs: links to the destination's staging folder, removed when the copy finishes or on the next launch. The copied file itself goes to its destination as it would without Dupin.
- An update download in a hidden `.Dupin-update-` folder beside the app, the extensions folder's package code, and the control socket in the per-user temporary folder.

## What a namespace holds

- **Records** are JSON values under a stable 128-bit ID. `put` stores a new record under a random ID; `upsert` stores the record for an owner's logical key, replacing the one stored before. The ID comes from the key through a keyed hash (`key_id`), so the key itself is never written. Every write is a new version stamped by the device's hybrid logical clock, and the record keeps the wall-clock time it was first stored, across replacements. `list` pages newest version first, with a `before` cursor (the last record's stamp and ID, which stays valid if that record is replaced or deleted), and returns at most 200 records a page.
- **Blobs** are byte strings stored beside a record: an image, a file snapshot, a video. A blob is named by a keyed hash of its contents, so the same image copied twice is stored once and kept until no record refers to it. `put` returns a handle per blob; `read_blob` takes the handle and returns the bytes.
- **Deleting** a record writes a tombstone version in its place, and **clearing** a namespace writes a clear marker that every older version falls under. A namespace that does not sync drops the tombstone or marker as soon as it is durable, so nothing is left behind; if Dupin stops in between, the next open finishes the job.
- **Limits** apply per namespace: maximum age (any duration, or forever), maximum entries, maximum total bytes and maximum bytes per item. An item over its limit is refused whole, never truncated. Age is measured from when a record was first stored; the entry and byte limits remove the oldest versions first. Retention deletes files outright and never writes tombstones. Limits apply when the namespace opens, after every `put` and when they change; the owner also runs a `Sweeper` thread that applies them periodically, so entries age out while nothing is being written. Hard ceilings (100,000 entries, 512 MiB per item, 16 GiB per namespace) bound what any owner can ask for.

`put` makes room for a new record by deleting the oldest ones. `put_within_limits` refuses a record there is no room for with `QuotaExceeded` and deletes nothing but records past their age; it checks the room and writes under one lock, so writers at the same time can't pass the limits together. Extensions store with it, since their quota is a promise not to take more, not a retention policy.

Every call blocks on file I/O and must run on a worker thread, never the UI thread. A `Store` can be shared between threads; calls on it are serialized. Only one `Store` per namespace can be open at a time, in this process or another, which an advisory lock on the namespace enforces.

## Extension namespaces

The broker bridge names an extension's namespace `ext-u-<hash>` for a package the user installed and `ext-b-<hash>` for one bundled with Dupin, where the hash is the first 160 bits of SHA-256 over the origin and the package ID. Native features use plain names without the `ext-` prefix, so the two never meet; neither do two packages, nor a user's copy of a bundled package and the bundled package itself, even when the copy is unmodified. The namespace opens with the quota and retention the package declares, bounded by Dupin's maxima (64 MiB, 10,000 records, 1 MiB per record, 3,650 days), stays open while Dupin runs, and a sweeper applies its retention hourly; every request also applies it first. Revoking `storage` keeps the namespace; `Bridge::delete_data`, which Settings' Delete Data, `dupin ext remove` and a fresh `dupin ext install` of the name call, discards it: it waits for requests already running, and a request that opened the namespace before then fails instead of writing into it.

## Built-in namespaces

A native feature opens its namespace by the name the [sync design](../design/sync.md#what-syncs) gives it, so sync can later carry its records without a migration. Each shares the launcher's one `Storage` with the extensions' namespaces, so all of them follow one device clock.

- **`snippets`**: the [snippets](./snippets.md) you add in Dupin, one record per snippet upserted under its keyword, holding `{ keyword, name, text }`. A record whose keyword doesn't name it is left out. Its limits leave room for every snippet Dupin lists (10,000), and writes use `upsert_within_limits`, so storing one never evicts another. Dupin opens it on a worker thread at launch only when the directory exists, so the data key is first read when you store a snippet, and a failure to open it is reported in the panel and retried on the next edit, never answered by deleting anything. It doesn't sync yet.
- **`snapshots`**: [saved snapshots](./snapshots.md), one record per snapshot upserted under its ID, holding the snapshot as the Snapshots window shows it. A record whose ID doesn't name it is left out. Its limits are the feature's own (100 snapshots, 512 KiB each, 8 MiB in all), and writes use `upsert_within_limits`, so a full store refuses a save rather than evicting one. One store per process owns it, on the snapshot writer's thread and the `snap` keyword's worker; it opens only when the directory exists, and a failure to open it is shown in the Snapshots window and on every Settings page and retried on the next save. It never syncs.

## On disk

The host roots the store at `storage/` in the writable data directory that also holds `settings.toml`: `~/Library/Application Support/Dupin/storage`, or `$XDG_DATA_HOME/dupin/storage` when that is set. Directories are created `0700` and files `0600`.

```
storage.state  the device's ID and clock, beside storage/ (and storage.key)
storage/<namespace>/
  lock         advisory lock while the namespace is open
  key.check    a known value sealed with the namespace key
  records/     <record id>.rec, the current version of each record
  blobs/       <keyed hash>.blob, one file per distinct blob
  tmp/         writes in progress
```

The format is version 2, laid out so that [sync between Macs](../design/sync.md#required-changes-to-dupin-storage) can later copy these files into a shared folder without a migration. Every sealed file starts with its magic, the format version and a plaintext **key tag** naming the key that sealed it: the device's data key, or a sync vault's ID and key epoch. The tag is also authenticated.

Keys are derived with HKDF-SHA256 (salt `dupin-storage v2`) from the key the tag names, with info naming the purpose, the key source and the namespace: a sealing key, a blob-naming key, a blob-nonce key, a name key for logical record keys, and a namespace ID, which is the name a synced folder would know the namespace by. Local directories keep the plain namespace name.

- **Records** are sealed under a fresh random 192-bit nonce. A record file holds the ID of the device that wrote the version in plaintext, then a sealed header (the version stamp, the creation time, whether it is a tombstone, the body's length and the blobs it uses), then the sealed JSON body, which a tombstone omits. Opening a namespace reads only the headers. The associated data authenticates the format version, the key tag, the namespace ID, the origin device and the record ID, and the body is bound to its own header; it names the record's identity, not the directory it sits in. A file renamed, moved to another namespace or rolled into another record's place fails to open.
- **Blobs** are named by a keyed hash of their contents and sealed under a nonce derived from that hash with the blob-nonce key, so the same content is always sealed to the same bytes. Their associated data authenticates the key tag, the namespace ID and the hash.
- **Version stamps** are the device's hybrid logical clock (Unix milliseconds in the upper 48 bits, a counter in the lower 16) and its device ID, which breaks ties. A write's stamp is later than every stamp the device has written or found on disk, even when the wall clock has gone back.
- **`storage.state`** sits beside the storage root, never inside it, so every name under `storage/` stays a namespace. It is sealed under the data key and holds the clock, saved before each stamp is used, and a random 128-bit device ID. When the host supplies the machine's hardware identity, the state also keeps a hash of it; a data directory that turns up on other hardware mints a new device ID and remembers the previous one. Opening a namespace also moves the clock past every stamp the namespace holds. The state is read only once a namespace's key check has passed. That check also passes for a new namespace, so it proves nothing about the key; a state that fails to open is therefore replaced with a new device ID only when no namespace holds data the key fails to open, and some namespace's check or record opens or no namespace holds files this version cannot judge. Otherwise the open reports a wrong key and the state is kept for the key it was sealed under. A state in a newer format is never replaced: every open reports that the data was written by a newer version, and nothing is changed.

A header field this version does not know is ignored, and a file in a newer format, or tagged with a key this device does not hold, is skipped: left on disk untouched, not listed, and counted in the open report. While any record is skipped, every blob found when the namespace opened is kept, even once no record of this key uses it, since a skipped record's blobs cannot be told apart from this key's; kept blobs are still shared by new records and count towards the namespace's byte limit. A blob first written while the namespace is open is removed as usual once nothing uses it.

Every write goes to `tmp/`, is flushed, then renamed into place and the directory flushed, so a crash leaves either the old state or the new one. There is no separate index to fall out of step: the record files are the index. Opening a namespace deletes leftover temporary files, records that fail authentication or have lost a blob, and blobs that no record uses. A damaged record is dropped on its own; the rest stay readable. Before anything is deleted as damaged, the key must be confirmed: the namespace's `key.check`, which is tagged like every other file, opens, or, if that file is missing or damaged, at least one record opens and the check is written again. A `key.check` in a newer format, or tagged with a key this device does not hold, is never judged or replaced: the records alone confirm the key, and a namespace holding only files this version cannot read opens without deleting anything. A wrong or missing key therefore reports an error and leaves every file untouched; the owner can then discard the namespace deliberately. A read or permission error is reported as an error too, never taken for damage. A blob that turns out to be damaged when read is dropped with the records that use it, so the next copy of the same content is stored afresh.

## The data key

One random 256-bit data key protects every namespace. A `KeyProvider` supplies it, and creates it only if none exists; a locked or refused key store is reported as an error, never answered with a new key that would orphan the stored data.

- **macOS:** a generic password item, service `app.dupin.Dupin.storage`, account `data-key`, never synchronized. A build with the data protection keychain entitlement stores it there as accessible after first unlock, this device only. Builds without that entitlement (ad hoc and Developer ID builds without a provisioning profile) fall back to the login keychain, which ignores the accessibility class and whose access control trusts the app that created the item, so a differently signed build may be asked for the login password before it can read the key.
- **Linux and others:** `FileKey`, a `0600` file meant to sit at `storage.key` beside `storage/`, never inside it, where every name belongs to a namespace. It is refused if other users can read it. This is weaker than a key store: anyone who can read the file, or a backup that contains it alongside the data, can decrypt everything.
- **Tests and fixtures** use an in-memory key and never touch the real Keychain.

## Threat model

Encryption at rest protects stored data from other user accounts on the Mac, from backups and synced copies of the data directory that do not include the Keychain, and from a lost or stolen disk. It also detects tampering: a modified, truncated, swapped or renamed file is rejected rather than read.

It does not protect against code running as the same user. Anything that can read the Keychain item, or that runs inside Dupin, can read everything stored. File sizes and modification times are not hidden: blob sizes and when each record last changed are visible to anyone who can list the directory. Record names are random or keyed hashes, so they reveal neither when a record was stored nor an owner's logical keys. The device ID in each record file is not a secret.

Deleting a record replaces its file with a tombstone, then removes that, and does not overwrite the old contents, because without the key the remains are ciphertext. Dupin zeroizes plaintext buffers it owns once it has used them: serialized JSON, blob contents passed to `put`, decrypted bodies and blobs, and key material. Values handed to callers, such as a parsed JSON record or a blob returned for display, are the caller's to clear, and copies made by the operating system, the allocator or Core Foundation are outside Dupin's reach. Zeroizing narrows how long plaintext stays in memory; it is not a guarantee against memory inspection.
