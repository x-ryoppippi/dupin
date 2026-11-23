# Clipboard history

Turn on clipboard history in Settings → Search (it is off by default; see [configuration](#configuration)). Then type `clip` and Space: a "Clipboard" chip shows the mode and the list starts with your most recent copies, each with how long ago it was copied. Type to keep only entries containing every word, ignoring case. ↑ and ↓ move through the list even with an empty query. Clipboard search terms are not kept for query recall.

Each row is text for now:

- **Text** shows its first words, with its line or character count. Words match anywhere in the text.
- **Images** show as "Image · 1280×720 PNG", with their size and the app they were copied from. They match `image`, `png`, their dimensions (`1280x720` or `1280×720`), their size (`1.2 mb`) and that app's name.
- **Files** show their names, with their folder and size. They match their names and full paths. A file that has since been moved or deleted shows as **Unavailable** and can only be deleted from the history.

Enter copies the selected entry back to the clipboard as it was copied — the text, the image, or the files as Finder's Copy offers them — and hides the panel. ⌘Enter pastes it into the frontmost app instead, which needs Accessibility access; without it the entry is copied and a notice explains how to allow it. The action screen (→, ⌘K or Ctrl+K) also offers Reveal in Finder for files, Delete from History and Clear History. Deleting removes that entry only; the clipboard itself keeps whatever it holds. Clear History asks for confirmation with Cancel selected before forgetting every entry. Clipboard results can't be saved as snapshots and are not recorded in usage-ranking history.

## Configuration

Clipboard history is off by default because it records what you copy. Turn it on with the switch in Settings → Search, or in `config.toml`. Below it, Settings sets how long copies are kept and whether images and files are recorded; when `[clipboard]` is set in a read-only config layer, these rows are locked.

```toml
[clipboard]
enabled = false # keep what you copy, searchable with `clip`
retention = "7d" # 1d, 7d, 30d, 90d or forever
include_images = true # images and screenshots, converted to PNG
include_files = true # files copied in Finder, kept by their location
max_entries = 200 # 1 to 1000; the oldest entry goes first
max_entry_bytes = 65536 # 1024 to 1048576; larger text is skipped, never truncated
max_image_bytes = 4194304 # 65536 to 33554432; the largest image kept, after converting it to PNG
ignored_apps = [
	"com.apple.keychainaccess",
	"com.apple.Passwords",
	"com.1password.1password",
	"com.agilebits.onepassword7",
	"com.bitwarden.desktop",
]
```

While it is on, Dupin checks the clipboard's change count twice a second on a background thread and records each new copy as one kind: the files copied in Finder, else plain text, else an image. Rich text is recorded as its plain text. Copies already on the clipboard when the history is turned on are not recorded. Copying something already in the history moves it to the top.

- **Images** are read from PNG, TIFF, HEIC, HEIF or JPEG on the clipboard and kept as PNG. An image larger than 4096 pixels on its longest side is scaled down to it; one whose PNG is still larger than `max_image_bytes` is skipped. A PNG that already fits is kept byte for byte. Images share 128 MB of memory; past that, the oldest images go first.
- **Files**, including videos, are never copied. Dupin keeps each file's path, name, size and type, so a file larger than the history is never duplicated. No thumbnail is made.
- **Text** larger than `max_entry_bytes` is skipped rather than kept in part.

Turning off `include_images` or `include_files` forgets the entries of that kind already kept, and copies of that kind are then not read at all. Entries older than `retention` are deleted as soon as the history notices: when a copy is recorded, the settings change or you search it.

Copies that password managers and other apps mark as concealed, transient or auto-generated ([nspasteboard.org](http://nspasteboard.org) types) are never read. Nor are copies that name an app in `ignored_apps` as their source, or that land while such an app is frontmost: Dupin checks the apps frontmost at the polls just before and just after a copy, so switching apps several times within half a second can slip past it. Rely on the concealed marker, which password managers set, for secrets. Setting `ignored_apps` replaces the defaults; find an app's identifier with `osascript -e 'id of app "Name"'`.

On macOS versions that report the pasteboard access policy, background recording requires **Always Allow**. Under the default policy, `clip` offers a one-time clipboard read so macOS can prompt and list Dupin in System Settings. If macOS is set to ask or deny, the row opens Privacy & Security, where you can always allow Dupin to paste from other apps.

## Storage

The history lives only in Dupin's memory for now. It is not written to disk, included in snapshots or logged, and it is gone when Dupin quits, so retention only matters while Dupin keeps running. The history writes every change through to a store, so it can persist across restarts once Dupin's encrypted storage lands; it will not be written to disk unencrypted.

Deleting an entry, clearing the history and turning it off delete what the store holds as well as what is in memory. Text and image data are overwritten with zeros in memory once the last copy of an entry is dropped. A row that is on screen holds its own copy of the text until the list is refreshed.
