# Configuration

Settings (⌘, in the panel) changes the hotkey, where the panel opens, the input source and how many files a search shows. Changes apply at once and are written to `config.toml`, replacing any comments in it.

Dupin reads `$XDG_CONFIG_HOME/dupin/config.toml`, or `~/.config/dupin/config.toml` when `XDG_CONFIG_HOME` is unset. Every key is optional; a missing file means the defaults. Dupin reads the file at launch, so restart it after editing it by hand.

```toml
hotkey = "cmd+space" # modifiers cmd, alt, ctrl, shift, joined with + to a key
file_limit = 200 # the most files a Spotlight search shows
position = "center" # "center" puts the field at the middle of the display; "top" near its top
input_source = "ascii" # "ascii", "keep", or an input source identifier such as "com.apple.keylayout.ABC"
```

An unknown key or an invalid value makes Dupin fall back to the defaults and say why, in the log and in the panel.

## Input source

Like Alfred's "Force Keyboard", Dupin switches the keyboard input source while the panel shows and switches back when it hides, so a Japanese or other input method doesn't hold the query in composition. `"ascii"` picks the most recently used source that types ASCII (ABC, or an input method's Roman mode); an identifier picks that source; `"keep"` leaves the input source alone.

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
