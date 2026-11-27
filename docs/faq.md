# FAQ

Short answers, each with the page that has the details.

## Getting started

<details>
<summary>What is Dupin?</summary>

A light, always-ready launcher for the Mac. Press the hotkey, type a few letters or a keyword, and press ↩. See [Using Dupin](./usage.md) and the [Philosophy](./philosophy.md).

</details>

<details>
<summary>Which Macs and macOS versions does it run on?</summary>

Apple silicon Macs on macOS 14 or later. See the [download page](https://dupin.app/download/).

</details>

<details>
<summary>Is there a Windows or Linux version?</summary>

No, Dupin is for macOS only. A Linux port is deferred and Windows is out of scope ([Sync design](./design/sync.md)).

</details>

<details>
<summary>How do I install and update it?</summary>

Install the Homebrew cask or build it with Nix; the [download page](https://dupin.app/download/) has both. Release builds update themselves, and **Update Dupin** in the main search checks at once ([Updates](./features/updates.md)). A Nix build is updated through Nix.

</details>

<details>
<summary>How do I uninstall it?</summary>

Remove the app the way you installed it. Your configuration, encrypted data and its Keychain key stay until you delete them, which deletes the data for good ([Uninstalling](./uninstall.md)).

</details>

<details>
<summary>Why doesn't <kbd>⌘</kbd><kbd>Space</kbd> open Dupin?</summary>

macOS delivers a chord to only one app, so Spotlight's shortcut, or another launcher's, has to give it up first. [Freeing the hotkey](./features/hotkeys.md#freeing-the-hotkey) lists the steps, and Dupin reports a taken hotkey in the panel and its log.

</details>

<details>
<summary>How do I see every keyword?</summary>

Type <kbd>?</kbd> in the panel. [Keywords](./features/keywords.md) lists them.

</details>

<details>
<summary>Can I add my own shortcuts and aliases?</summary>

Yes. `[aliases]` in `config.toml` adds your own shorthands for a command, an app, a web search or a file, and global shortcuts open a command from any app ([Aliases](./features/keywords.md#aliases), [Shortcuts](./features/keywords.md#shortcuts)).

</details>

## Privacy and data

<details>
<summary>Does Dupin send any data?</summary>

Official releases send one anonymous ping a day with a random ID, the version, the channel, the macOS major version and the CPU architecture, and never anything you search or open. Turn it off in Settings or with `telemetry = false`; Nix and source builds never send it ([Telemetry](./features/telemetry.md)).

</details>

<details>
<summary>Where is my data kept, and is it encrypted?</summary>

On your Mac, encrypted on disk under a key in your Keychain, with no account ([Encrypted storage](./features/storage.md)).

</details>

<details>
<summary>Can I sync between Macs?</summary>

Yes, through a folder another tool already syncs, such as iCloud Drive, end-to-end encrypted, with no Dupin server. It carries usage history today ([Sync](./features/sync.md)).

</details>

<details>
<summary>Which macOS permissions does it ask for, and when?</summary>

Accessibility, Calendars and Paste from Other Apps, only when a feature that needs one is turned on or used. Starting Dupin, opening Settings and reloading never ask ([Permissions](./features/permissions.md)).

</details>

## Extensions

<details>
<summary>How do I write an extension?</summary>

An extension is a TypeScript or JavaScript package. `dupin ext new` creates one, and `dupin ext test` and `dupin ext snap` check it ([Developing and testing extensions](./features/extension-testing.md), [Extensions](./features/extensions.md)).

</details>

<details>
<summary>Can an AI agent write one?</summary>

Yes, an extension is plain text an agent can write and check. `dupin mcp --workspace DIR` gives an agent the check, run, test and snapshot tools for packages in `DIR` ([MCP server](./features/mcp.md#extension-tools), [Testing through MCP](./features/extension-testing.md#through-mcp)).

</details>

<details>
<summary>Are extensions safe?</summary>

Each runs in a sandboxed helper process that can't open files, use the network or start programs by itself. It reaches only the capabilities it declares and you allow, and you can revoke them, but install only extensions you trust ([Extension capabilities](./features/extension-permissions.md#security-model)).

</details>

<details>
<summary>Is there an extension store? How do I find extensions?</summary>

There is no store. Extensions are installed from Git, and `dupin ext search` lists public GitHub repositories with the `dupin-extension` topic, marked unreviewed ([Finding packages](./features/extension-distribution.md#finding-packages)).

</details>

<details>
<summary>Can I use npm packages in an extension?</summary>

Yes, bundle them into one module with `dupin ext build`, which uses Rolldown. Node built-ins such as `fs` aren't available, and a dependency that imports one fails the build ([Building a bundle](./features/extension-distribution.md#building-a-bundle)).

</details>

## Comparisons

<details>
<summary>How is Dupin different from Alfred?</summary>

Dupin continues Alfred's keyword-first way of working: a short keyword and Space reaches anything. Its extensions are plain text, so an agent can write them as well as you can ([Philosophy](./philosophy.md)).

</details>

<details>
<summary>How is Dupin different from Raycast?</summary>

Dupin chooses to stay a launcher, the doorway to your desktop, and grows through small extensions rather than becoming a place where programs run. Its extensions are plain TypeScript that Dupin draws itself, installed from Git ([Philosophy](./philosophy.md)).

</details>

<details>
<summary>Will AI or a better Spotlight make launchers unnecessary?</summary>

The system launcher keeps improving and suits many people. Dupin is for those who want keyword-first, predictable, extensible control: when you know what you want, a few keys beat describing it. AI changes how tools are built, not whether you want them, since an agent can write an extension once and you then run it without a model ([Philosophy](./philosophy.md)).

</details>

## Project

<details>
<summary>Is Dupin open source?</summary>

Yes, under [GPL-3.0-or-later](https://github.com/x-ryoppippi/dupin/blob/main/LICENSE). Contributions are made under the [CLA](https://github.com/x-ryoppippi/dupin/blob/main/CLA.md) ([Contributing](https://github.com/x-ryoppippi/dupin/blob/main/CONTRIBUTING.md)).

</details>

<details>
<summary>How fast is it?</summary>

App search shows its results under 3 ms after each keystroke at the 95th percentile, measured in CI on every change to the app ([Philosophy](./philosophy.md#fast-by-rule), [Keypress latency](./development.md#keypress-latency)).

</details>

<details>
<summary>Why is it called Dupin?</summary>

After C. Auguste Dupin, Poe's detective and the source of the line Sherlock, Watson and Alfred come from ([The name](./name.md)).

</details>

<details>
<summary>Where do I report bugs or ask questions?</summary>

On [GitHub issues](https://github.com/x-ryoppippi/dupin/issues).

</details>
