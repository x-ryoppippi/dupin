# Philosophy

Dupin is what Alfred was meant to be: a launcher that stays simple and that anyone can extend. [The name](./name.md#why-dupin-was-built) tells why its author built it after Alfred and Raycast.

## Built for searching

Press the hotkey, type a few letters and press Enter. A short [keyword](./features/keywords.md) and Space reaches anything else, such as `cal` for your calendar or `d` for the dictionary, and `?` lists every one. That is the whole interface, and it stays that way.

## Fast, by rule

Dupin stays running in the background, so the panel is there the moment you press the hotkey. In CI, on every pull request that changes the app, a keystroke reaches the first frame that shows its results in under 3 ms at the 95th percentile for the main search, and in under 3.5 ms for every search measured. A bundled extension that falls behind the native code it replaced can't merge ([Keypress latency](./development.md#keypress-latency)).

## Extended with plain TypeScript

An [extension](./features/extensions.md) is a TypeScript or JavaScript package: text that you, or an agent, can write, check, test against fixtures and render from the command line, and install from Git. There is no store. Each one runs sandboxed and reaches only what you allow it. The calculator, calendar, clipboard history, dictionary and the other [bundled extensions](./features/extensions.md#bundled-extensions) are built the same way, and you can eject any of them to change it.

There is no account, and what Dupin keeps is [encrypted](./features/storage.md) on your Mac. Dupin is for macOS on Apple silicon.
