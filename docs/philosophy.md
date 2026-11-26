# Philosophy

Dupin is a launcher in Alfred's line, meant to succeed it. [The name](./name.md) tells that story, and why its author built Dupin after using Alfred and Raycast. These are the choices it makes.

## Keywords first, and light

You press the hotkey, type a few letters and press Enter. A short [keyword](./features/keywords.md) and Space reaches a command, such as `cal` for your calendar or `d` for the dictionary, and `?` lists every one. Dupin is a small native app that stays running in the background, with no Dock icon or menu bar, so the panel is there the moment you press the hotkey.

## Extensions are text

An [extension](./features/extensions.md) is a TypeScript or JavaScript package: plain text that you, or an agent, can write, check, test against fixtures and render to a PNG from the command line (`dupin ext check`, `dupin ext test`, `dupin ext snap`). There is no store and no review. You install a package from Git, pinned to a commit.

Each extension runs in a sandboxed helper process and can reach nothing on its own. It declares what it needs, such as one website or a folder you pick, and you allow each one and can revoke it later.

The calculator, calendar, clipboard history, dictionary and other [bundled extensions](./features/extensions.md#bundled-extensions) are built the same way, and you can eject any of them to change it.

## Local first

There is no account. What Dupin learns and keeps about you, such as your usage history, clipboard history, snippets and extensions' data, is [encrypted on disk](./features/storage.md) under a key in your Keychain. Your own `config.toml` stays a plain file you can edit or generate.

[Sync](./features/sync.md) goes through a folder you already sync, such as iCloud Drive or Dropbox, end-to-end encrypted, with no Dupin server.

## Made for the Mac

Dupin runs only on macOS, as a native app for Apple silicon Macs. It builds on what macOS already has, such as Spotlight for files and the system dictionaries for definitions, rather than reinventing them.
