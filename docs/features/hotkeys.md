# Hotkeys

Global shortcuts show the panel, open one of Dupin's commands or web searches, run a system command or an extension's command, launch an app, or run a shell command or script from any app.

## Open it

Press the chord from any app: <kbd>⌘Space</kbd> for the panel by default, or one you set in [`bindings`](#configuration) or Settings → Keywords.

## What you see

- A Dupin binding to a command or a web search shows the panel already in it, such as clipboard history for <kbd>⇧⌘V</kbd>, from whatever it showed before.
- A Dupin binding to a row, such as **Sleep** or Running Apps' **Quit** for one app, shows nothing: the row runs as choosing it does. The panel shows only for what the row has to say, or for a row that asks first or opens something to choose from.
- In `?` and as you type, a command's row shows its shortcut beside its keywords.
- Settings → General lists the configured bindings.

## What you can do

| Key / action                         | What it does                                                                                                      |
| ------------------------------------ | ----------------------------------------------------------------------------------------------------------------- |
| The panel hotkey (<kbd>⌘Space</kbd>) | Show or hide the panel                                                                                            |
| A `dupin` binding to a command       | Show the panel in that command, optionally with a query typed; press again while it shows that command to hide it |
| A `dupin` binding to a web search    | Show the main search with the web search's keyword and the query typed; press again to hide it                    |
| A `dupin` binding to a row           | Run the row at once, as <kbd>↩</kbd> on it does                                                                   |
| A `dupin` binding to an app or file  | Open it, as <kbd>↩</kbd> on its alias's row does                                                                  |
| An `app` binding                     | Launch the app or bring it to the front; with `hide_if_frontmost = true`, hide it if it is already frontmost      |
| A `command` binding                  | Run a shell command line silently in the background, such as a CLI                                                |
| A `script` binding                   | Run a [script command](./scripts.md#hotkeys) with optional arguments                                              |

Other keys: [Keys](../usage.md#keys).

## Notes

- macOS delivers a chord to only one app, so [free the hotkey](#freeing-the-hotkey) elsewhere first; ⌘Space belongs to Spotlight by default.
- Each shortcut must be distinct from the panel and selection shortcuts and other bindings. Dupin reports clashes, unavailable commands and invalid script bindings in Settings and in the panel.
- A binding whose command isn't available, because its feature is off, no extension that is on has that package name, no web search answers to its keyword, or its system command needs **System Command Automation** or isn't in this build, is skipped with a notice that names it; the rest of the configuration applies. Turning the feature, the extension or the setting on makes the shortcut work without a restart.
- Restart, Shut Down, Log Out and Empty Trash never run straight from a shortcut: it shows their row in the main search, and <kbd>↩</kbd> asks as it always does. A row of an extension's that asks first, or opens a list or a view, shows its package's command with the row's title typed, the same way.
- An extension's row is looked up when you press the shortcut, so a row the package no longer lists is reported then.
- When an extension's row needs access its package hasn't been given, such as a scope you haven't allowed or Accessibility, the shortcut shows the panel in that package's command with the request above the results. Once you allow it, the row runs, as long as that is within two minutes of the press; **Deny** drops it.
- Up to four shell commands can run at a time.
- Edits reload automatically.

## Configuration

```toml
hotkey = "cmd+space" # modifiers cmd, alt, ctrl, shift, joined with + to a key
```

An empty `hotkey` disables the panel shortcut. Add a `[bindings."<chord>"]` table for each other shortcut:

| `target`    | Keys                                      | Meaning                                                                                                                                                                               |
| ----------- | ----------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `"dupin"`   | `command`, optional `query`               | What an [alias](./keywords.md#aliases) reaches, written as an alias's target is, or one [row](#rows) as `<command>/<row>`; `query` is typed into a command's or a web search's search |
| `"app"`     | `bundle_id`, optional `hide_if_frontmost` | An app by its macOS bundle identifier; `hide_if_frontmost` defaults to `false`                                                                                                        |
| `"command"` | `command`                                 | A shell command line                                                                                                                                                                  |
| `"script"`  | `script`, optional `args`                 | A [script](./scripts.md#hotkeys) by its executable's filename, including its extension, with an array of arguments                                                                    |

```toml
[bindings."cmd+shift+v"]
target = "dupin"
command = "clipboard" # ⇧⌘V opens clipboard history

[bindings."alt+cmd+d"]
target = "dupin"
command = "dictionary"
query = "serendipity" # optional: typed into the command's search

[bindings."alt+cmd+s"]
target = "dupin"
command = "system/sleep" # ⌥⌘S puts the Mac to sleep

[bindings."ctrl+alt+q"]
target = "dupin"
command = "running-apps/com.apple.Safari" # an extension's row, by its package and ID

[bindings."alt+cmd+y"]
target = "dupin"
command = "web:yt" # the YouTube search, ready for a query

[bindings."ctrl+alt+c"]
target = "app"
bundle_id = "com.apple.calculator"
hide_if_frontmost = true # optional, defaults to false

[bindings."alt+cmd+g"]
target = "command"
command = "say 'Back to work'"

[bindings."alt+cmd+t"]
target = "script"
script = "echo-message.sh"
args = ["Hello from Dupin"]
```

Settings → Keywords records Dupin bindings for its commands, web searches, aliased apps and files, and system commands (see [Keywords › Settings](./keywords.md#settings)).

### Rows

| `command`         | Runs                                                                                                                                                                                        |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `system/<row>`    | A [system command](./system-commands.md): `sleep`, `lock-screen`, `sleep-displays`, `toggle-dark-mode`, `restart`, `shut-down`, `log-out`, `empty-trash`, `restart-dupin` or `update-dupin` |
| `<package>/<row>` | The row an extension's `items()` catalogue lists with that `id`, such as `running-apps/com.apple.Safari`, which quits Safari; see [Running apps](./running-apps.md)                         |

A system command's row is its title in lowercase with hyphens for spaces. An extension's row ID is the `id` its [`items()`](./extension-api.md#itemsctx) gives it; a package with only `search` has no rows to bind. The rows of a built-in command, such as clipboard entries or calendar events, can't be bound.

> [!TIP]
> To open the panel in one of Dupin's commands, use a `dupin` binding rather than a shell command: one such as `dupin open --query 'clip '` starts a shell and the CLI on every press and takes one of the four command slots.

<details>
<summary>Dupin bindings</summary>

The panel shows the command at once, without starting a process. Pressing the shortcut again while the panel shows that command hides it, as the panel's own hotkey does.

A command name is lowercase letters, digits and hyphens, a row ID after the slash is 1 to 256 bytes on one line, and a query is at most 1024 bytes on one line; anything else makes `config.toml` invalid. A `query` beside a row, an app or a file makes the configuration fail to apply, naming the shortcut.

</details>

<details>
<summary>Shell commands</summary>

Dupin runs commands in the background from your home directory using `$SHELL -lc` (with `/bin/zsh` as the fallback). Standard input is closed. Successful commands run silently. On failure, Dupin shows a notice with the error or exit status and up to four lines from the end of stderr. Up to four commands can run at a time.

Commands run until they exit or Dupin quits or relaunches. Automatic updates can restart Dupin while commands are running. If all four slots are in use, wait for a command to finish or quit and reopen Dupin to clear the slots. A background job can keep running after its shell exits, which frees the command slot. Redirect that job's standard error if it needs to keep writing after Dupin quits.

</details>

<details>
<summary>Home Manager</summary>

Declare bindings in `programs.dupin.settings.bindings`:

```nix
programs.dupin.settings.bindings."cmd+shift+v" = {
  target = "dupin";
  command = "clipboard";
};
```

`bindings` is then set in a read-only `config.toml`, so Settings shows the recorders on Keywords locked, **Managed by Nix**. If the script is missing, unavailable or its arguments are invalid, Dupin skips that binding and applies the rest of the configuration. Fixing the script or binding makes the shortcut available automatically.

</details>

## Freeing the hotkey

macOS delivers a chord to only one app, so free Dupin's hotkey elsewhere first. For the default ⌘Space:

1. Turn off Spotlight's shortcut: System Settings → Keyboard → Keyboard Shortcuts… → Spotlight → untick "Show Spotlight search".
2. Remove ⌘Space from any other launcher that uses it, such as Raycast (Settings → General → Raycast Hotkey) or Alfred.

⌃Space is macOS's "Select the previous input source" shortcut (System Settings → Keyboard → Keyboard Shortcuts… → Input Sources), so free it there before using it.

<details>
<summary>When the hotkey is taken</summary>

If the hotkey can't be registered, Dupin doesn't fail silently. It:

- writes the reason to stderr and to `~/Library/Logs/Dupin/dupin.log`, where Console.app shows it;
- posts a notification, when it runs as the bundled app;
- opens the panel with the message, since it has no other window.

It also names a macOS shortcut that holds the chord (Spotlight's, for one), which registers without an error but would take the key first.

One case can't be detected: another app holding the chord through a non-exclusive registration, as Raycast and Alfred can. macOS accepts both registrations and delivers the key to only one of them, which is why step 2 above matters.

</details>
