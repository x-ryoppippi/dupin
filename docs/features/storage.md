# Encrypted storage

`dupin-storage` is the host's store for data that must outlive a restart but should not sit on disk in the clear, such as clipboard history. Each owner opens its own namespace: a native feature by name (`clipboard`), and later an extension through the broker's `storage` capability, which maps the installed package to a namespace of its own. The owner never sees paths or keys; it stores JSON records and the blobs they refer to, and reads them back.

## What a namespace holds

- **Records** are JSON values under a stable 128-bit ID. `put` stores a new record under a random ID; `upsert` stores the record for an owner's logical key, replacing the one stored before. The ID comes from the key through a keyed hash (`key_id`), so the key itself is never written. Every write is a new version stamped by the device's hybrid logical clock, and the record keeps the wall-clock time it was first stored, across replacements. `list` pages newest version first, with a `before` cursor (the last record's stamp and ID, which stays valid if that record is replaced or deleted), and returns at most 200 records a page.
- **Blobs** are byte strings stored beside a record: an image, a file snapshot, a video. A blob is named by a keyed hash of its contents, so the same image copied twice is stored once and kept until no record refers to it. `put` returns a handle per blob; `read_blob` takes the handle and returns the bytes.
- **Deleting** a record writes a tombstone version in its place, and **clearing** a namespace writes a clear marker that every older version falls under. A namespace that does not sync drops the tombstone or marker as soon as it is durable, so nothing is left behind; if Dupin stops in between, the next open finishes the job.
- **Limits** apply per namespace: maximum age (any duration, or forever), maximum entries, maximum total bytes and maximum bytes per item. An item over its limit is refused whole, never truncated. Age is measured from when a record was first stored; the entry and byte limits remove the oldest versions first. Retention deletes files outright and never writes tombstones. Limits apply when the namespace opens, after every `put` and when they change; the owner also runs a `Sweeper` thread that applies them periodically, so entries age out while nothing is being written. Hard ceilings (100,000 entries, 512 MiB per item, 16 GiB per namespace) bound what any owner can ask for.

Every call blocks on file I/O and must run on a worker thread, never the UI thread. A `Store` can be shared between threads; calls on it are serialized. Only one `Store` per namespace can be open at a time, in this process or another, which an advisory lock on the namespace enforces.

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
- **`storage.state`** sits beside the storage root, never inside it, so every name under `storage/` stays a namespace. It is sealed under the data key and holds the clock, saved before each stamp is used, and a random 128-bit device ID. When the host supplies the machine's hardware identity, the state also keeps a hash of it; a data directory that turns up on other hardware mints a new device ID and remembers the previous one. Opening a namespace also moves the clock past every stamp the namespace holds. The state is read only once a namespace's key check has passed, and a damaged one is replaced with a new device ID.

A header field this version does not know is ignored, and a file in a newer format, or tagged with a key this device does not hold, is skipped: left on disk untouched, not listed, and counted in the open report. Unreferenced blobs are kept while any record is skipped, since its blobs cannot be told apart from orphans.

Every write goes to `tmp/`, is flushed, then renamed into place and the directory flushed, so a crash leaves either the old state or the new one. There is no separate index to fall out of step: the record files are the index. Opening a namespace deletes leftover temporary files, records that fail authentication or have lost a blob, and blobs that no record uses. A damaged record is dropped on its own; the rest stay readable. Before anything is deleted as damaged, the key must be confirmed: the namespace's `key.check`, which is tagged like every other file, opens, or, if that file is missing or damaged, at least one record opens and the check is written again. A wrong or missing key therefore reports an error and leaves every file untouched; the owner can then discard the namespace deliberately. A read or permission error is reported as an error too, never taken for damage. A blob that turns out to be damaged when read is dropped with the records that use it, so the next copy of the same content is stored afresh.

## The data key

One random 256-bit data key protects every namespace. A `KeyProvider` supplies it, and creates it only if none exists; a locked or refused key store is reported as an error, never answered with a new key that would orphan the stored data.

- **macOS:** a generic password item, service `app.dupin.Dupin.storage`, account `data-key`, never synchronized. A build with the data protection keychain entitlement stores it there as accessible after first unlock, this device only. Builds without that entitlement (ad hoc and Developer ID builds without a provisioning profile) fall back to the login keychain, which ignores the accessibility class and whose access control trusts the app that created the item, so a differently signed build may be asked for the login password before it can read the key.
- **Linux and others:** `FileKey`, a `0600` file meant to sit at `storage.key` beside `storage/`, never inside it, where every name belongs to a namespace. It is refused if other users can read it. This is weaker than a key store: anyone who can read the file, or a backup that contains it alongside the data, can decrypt everything.
- **Tests and fixtures** use an in-memory key and never touch the real Keychain.

## Threat model

Encryption at rest protects stored data from other user accounts on the Mac, from backups and synced copies of the data directory that do not include the Keychain, and from a lost or stolen disk. It also detects tampering: a modified, truncated, swapped or renamed file is rejected rather than read.

It does not protect against code running as the same user. Anything that can read the Keychain item, or that runs inside Dupin, can read everything stored. File sizes and modification times are not hidden: blob sizes and when each record last changed are visible to anyone who can list the directory. Record names are random or keyed hashes, so they reveal neither when a record was stored nor an owner's logical keys. The device ID in each record file is not a secret.

Deleting a record replaces its file with a tombstone, then removes that, and does not overwrite the old contents, because without the key the remains are ciphertext. Dupin zeroizes plaintext buffers it owns once it has used them: serialized JSON, blob contents passed to `put`, decrypted bodies and blobs, and key material. Values handed to callers, such as a parsed JSON record or a blob returned for display, are the caller's to clear, and copies made by the operating system, the allocator or Core Foundation are outside Dupin's reach. Zeroizing narrows how long plaintext stays in memory; it is not a guarantee against memory inspection.
