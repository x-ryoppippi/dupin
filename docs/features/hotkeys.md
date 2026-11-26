# Hotkeys

Use a global shortcut to open one of Dupin's commands, launch an app, bring it to the front, or run a shell command. A command shortcut shows the panel already in that command, such as ⇧⌘V for clipboard history, and pressing it again hides the panel. Set `hide_if_frontmost = true` to hide an app if it is already frontmost. A shell command runs silently in the background, so you can use it to run a CLI directly. See [hotkey configuration](#configuration).

## Configuration

```toml
hotkey = "cmd+space" # modifiers cmd, alt, ctrl, shift, joined with + to a key
```

An empty `hotkey` disables the panel shortcut.

Add a table to `config.toml` for each shortcut. A Dupin binding opens one of Dupin's commands, named as an [alias](./keywords.md#aliases) names it: `files`, `contents`, `calendar`, `clipboard`, `dictionary`, `snippets`, `snapshots`, or an extension's package name.

```toml
[bindings."cmd+shift+v"]
target = "dupin"
command = "clipboard" # ⇧⌘V opens clipboard history

[bindings."alt+cmd+d"]
target = "dupin"
command = "dictionary"
query = "serendipity" # optional: typed into the command's search
```

The panel shows the command at once, from whatever it showed before, without starting a process. Pressing the shortcut again while the panel shows that command hides it, as the panel's own hotkey does. Settings → Keywords records these shortcuts too (see [Keywords › Settings](./keywords.md#settings)), and the command's row in `?` and as you type shows its shortcut beside its keywords.

A command name is lowercase letters, digits and hyphens, and a query is at most 1024 bytes on one line; anything else makes `config.toml` invalid. A binding whose command isn't available, because its feature is off or no extension that is on has that package name, is skipped with a notice that names it, and the rest of the configuration applies. Turning the feature or the extension on makes the shortcut work without a restart.

An app binding uses the app's macOS bundle identifier.

```toml
[bindings."ctrl+alt+c"]
target = "app"
bundle_id = "com.apple.calculator"
hide_if_frontmost = true # optional, defaults to false
```

A command binding runs a shell command line.

```toml
[bindings."alt+cmd+g"]
target = "command"
command = "say 'Back to work'"
```

To open the panel in one of Dupin's commands, use a Dupin binding instead: a shell command such as `dupin open --query 'clip '` starts a shell and the CLI on every press and takes one of the four command slots.

Dupin runs commands in the background from your home directory using `$SHELL -lc` (with `/bin/zsh` as the fallback). Standard input is closed. Successful commands run silently. On failure, Dupin shows a notice with the error or exit status and up to four lines from the end of stderr. Up to four commands can run at a time.

Commands run until they exit or Dupin quits or relaunches. Automatic updates can restart Dupin while commands are running. If all four slots are in use, wait for a command to finish or quit and reopen Dupin to clear the slots. A background job can keep running after its shell exits, which frees the command slot. Redirect that job's standard error if it needs to keep writing after Dupin quits.

A [script binding](./scripts.md#hotkeys) uses the executable's filename, including its extension, and can pass an array of arguments.

```toml
[bindings."alt+cmd+t"]
target = "script"
script = "echo-message.sh"
args = ["Hello from Dupin"]
```

Each shortcut must be distinct from the panel and selection shortcuts and other bindings. Settings → General lists the configured bindings. Edits reload automatically. Dupin reports clashes, unavailable commands and invalid script bindings in Settings and in the panel.

With Home Manager, declare bindings in `programs.dupin.settings.bindings`:

```nix
programs.dupin.settings.bindings."cmd+shift+v" = {
  target = "dupin";
  command = "clipboard";
};
```

`bindings` is then set in a read-only `config.toml`, so Settings shows the recorders on Keywords locked, **Managed by Nix**. If the script is missing, unavailable or its arguments are invalid, Dupin skips that binding and applies the rest of the configuration. Fixing the script or binding makes the shortcut available automatically.

## Freeing the hotkey

macOS delivers a chord to only one app, so free Dupin's hotkey elsewhere first. For the default ⌘Space:

1. Turn off Spotlight's shortcut: System Settings → Keyboard → Keyboard Shortcuts… → Spotlight → untick "Show Spotlight search".
2. Remove ⌘Space from any other launcher that uses it, such as Raycast (Settings → General → Raycast Hotkey) or Alfred.

⌃Space is macOS's "Select the previous input source" shortcut (System Settings → Keyboard → Keyboard Shortcuts… → Input Sources), so free it there before using it.

## When the hotkey is taken

If the hotkey can't be registered, Dupin doesn't fail silently. It:

- writes the reason to stderr and to `~/Library/Logs/Dupin/dupin.log`, where Console.app shows it;
- posts a notification, when it runs as the bundled app;
- opens the panel with the message, since it has no other window.

It also names a macOS shortcut that holds the chord (Spotlight's, for one), which registers without an error but would take the key first.

One case can't be detected: another app holding the chord through a non-exclusive registration, as Raycast and Alfred can. macOS accepts both registrations and delivers the key to only one of them, which is why step 2 above matters.
