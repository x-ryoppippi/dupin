# Encrypted storage

`dupin-storage` is the host's store for data that must outlive a restart but should not sit on disk in the clear, such as clipboard history. Each owner opens its own namespace: a native feature by name (`clipboard`), and an extension through the broker's `storage` capability, which maps the installed package to a namespace of its own (see [Extension namespaces](#extension-namespaces)). The owner never sees paths or keys; it stores JSON records and the blobs they refer to, and reads them back.

## What a namespace holds

- **Records** are JSON values with an ID and the time they were stored, taken from the clock even if it has gone back since the last record. IDs sort by that time, so `list` pages newest first with a `before` cursor and returns at most 200 records a page.
- **Blobs** are byte strings stored beside a record: an image, a file snapshot, a video. A blob is named by a keyed hash of its contents, so the same image copied twice is stored once and kept until no record refers to it. `put` returns a handle per blob; `read_blob` takes the handle and returns the bytes.
- **Limits** apply per namespace: maximum age (any duration, or forever), maximum entries, maximum total bytes and maximum bytes per item. An item over its limit is refused whole, never truncated. The oldest records go first when a limit is exceeded. Limits apply when the namespace opens, after every `put` and when they change; the owner also runs a `Sweeper` thread that applies them periodically, so entries age out while nothing is being written. Hard ceilings (100,000 entries, 512 MiB per item, 16 GiB per namespace) bound what any owner can ask for.

`put` makes room for a new record by deleting the oldest ones. `put_within_limits` refuses a record there is no room for with `QuotaExceeded` and deletes nothing but records past their age; it checks the room and writes under one lock, so writers at the same time can't pass the limits together. Extensions store with it, since their quota is a promise not to take more, not a retention policy.

Every call blocks on file I/O and must run on a worker thread, never the UI thread. A `Store` can be shared between threads; calls on it are serialized. Only one `Store` per namespace can be open at a time, in this process or another, which an advisory lock on the namespace enforces.

## Extension namespaces

The broker bridge names an extension's namespace `ext-u-<hash>` for a package the user installed and `ext-b-<hash>` for one bundled with Dupin, where the hash is the first 160 bits of SHA-256 over the origin and the package ID. Native features use plain names without the `ext-` prefix, so the two never meet; neither do two packages, nor a user's copy of a bundled package and the bundled package itself, even when the copy is unmodified. The namespace opens with the quota and retention the package declares, bounded by Dupin's maxima (64 MiB, 10,000 records, 1 MiB per record, 3,650 days), stays open while Dupin runs, and a sweeper applies its retention hourly; every request also applies it first. Revoking `storage` keeps the namespace; `Bridge::delete_data`, which Settings' Delete Data, `dupin ext remove` and a fresh `dupin ext install` of the name call, discards it: it waits for requests already running, and a request that opened the namespace before then fails instead of writing into it.

## On disk

The host roots the store at `storage/` in the writable data directory that also holds `settings.toml`: `~/Library/Application Support/Dupin/storage`, or `$XDG_DATA_HOME/dupin/storage` when that is set. Directories are created `0700` and files `0600`.

```
storage/<namespace>/
  lock        advisory lock while the namespace is open
  key.check   a known value sealed with the namespace key
  records/    <id>.rec, one file per record
  blobs/      <keyed hash>.blob, one file per distinct blob
  tmp/        writes in progress
```

Each file is sealed with XChaCha20-Poly1305 under a fresh random 192-bit nonce. Two keys are derived per namespace from the data key with HKDF-SHA256: one seals files, the other names blobs. The associated data authenticates the format version, the namespace and the file's name, so a file renamed, moved to another namespace or rolled into another record's place fails to open. A record file seals a small header (the body's length and the blobs it uses) and the JSON body separately, so opening a namespace reads only the headers. The body is bound to its own header.

Every write goes to `tmp/`, is flushed, then renamed into place and the directory flushed, so a crash leaves either the old state or the new one. There is no separate index to fall out of step: the record files are the index. Opening a namespace deletes leftover temporary files, records that fail authentication or have lost a blob, and blobs that no record uses. A damaged record is dropped on its own; the rest stay readable. Before anything is deleted as damaged, the key must be confirmed: the namespace's `key.check` opens, or, if that file is missing or damaged, at least one record opens and the check is written again. A wrong or missing key therefore reports an error and leaves every file untouched; the owner can then discard the namespace deliberately. A read or permission error is reported as an error too, never taken for damage. A blob that turns out to be damaged when read is dropped with the records that use it, so the next copy of the same content is stored afresh.

## The data key

One random 256-bit data key protects every namespace. A `KeyProvider` supplies it, and creates it only if none exists; a locked or refused key store is reported as an error, never answered with a new key that would orphan the stored data.

- **macOS:** a generic password item, service `app.dupin.Dupin.storage`, account `data-key`, never synchronized. A build with the data protection keychain entitlement stores it there as accessible after first unlock, this device only. Builds without that entitlement (ad hoc and Developer ID builds without a provisioning profile) fall back to the login keychain, which ignores the accessibility class and whose access control trusts the app that created the item, so a differently signed build may be asked for the login password before it can read the key.
- **Linux and others:** `FileKey`, a `0600` file meant to sit at `storage.key` beside `storage/`, never inside it, where every name belongs to a namespace. It is refused if other users can read it. This is weaker than a key store: anyone who can read the file, or a backup that contains it alongside the data, can decrypt everything.
- **Tests and fixtures** use an in-memory key and never touch the real Keychain.

## Threat model

Encryption at rest protects stored data from other user accounts on the Mac, from backups and synced copies of the data directory that do not include the Keychain, and from a lost or stolen disk. It also detects tampering: a modified, truncated, swapped or renamed file is rejected rather than read.

It does not protect against code running as the same user. Anything that can read the Keychain item, or that runs inside Dupin, can read everything stored. File names, sizes and modification times are not hidden: a record's name is the time it was stored, and blob sizes are visible to anyone who can list the directory.

Deleting a record removes its file and does not overwrite it, because without the key the remains are ciphertext. Dupin zeroizes plaintext buffers it owns once it has used them: serialized JSON, blob contents passed to `put`, decrypted bodies and blobs, and key material. Values handed to callers, such as a parsed JSON record or a blob returned for display, are the caller's to clear, and copies made by the operating system, the allocator or Core Foundation are outside Dupin's reach. Zeroizing narrows how long plaintext stays in memory; it is not a guarantee against memory inspection.
