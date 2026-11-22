# Configuration

Settings (⌘, in the panel) changes the hotkey, where the panel opens, the input source, how many files a search shows and the backdrop. Changes apply at once and are written to `config.toml`, replacing any comments in it. A symlinked `config.toml` is written where it points, so the link stays; a read-only one, such as one [Nix manages](#nix), is shown in Settings but not changed there.

Saving from Settings merges the changed fields into the latest valid file, preserving unrelated external edits. If the same field changed differently in the file, or the file is invalid, Settings reports the problem and leaves the file untouched.

Dupin reads `$XDG_CONFIG_HOME/dupin/config.toml`, or `~/.config/dupin/config.toml` when `XDG_CONFIG_HOME` is unset. Every key is optional; a missing file means the defaults. File changes apply automatically while Dupin runs, including atomic replacements and changes to a linked file. The panel and Settings use the same configuration.

```toml
hotkey = "cmd+space" # modifiers cmd, alt, ctrl, shift, joined with + to a key
file_limit = 200 # the most files a Spotlight search shows
dictionary_keywords = ["define", "d"] # type any keyword, then Space, to look up a word
position = "center" # "center" puts the field at the middle of the display; "top" near its top
input_source = "ascii" # "ascii", "keep", or an input source identifier such as "com.apple.keylayout.ABC"
auto_update = true # whether a release build checks for and installs its channel's updates by itself
backdrop = "off" # "off", "flow", or the path to a WGSL shader
```

At launch, a parse error makes Dupin use the defaults and report the problem. During a reload, a parse error or unavailable new hotkey keeps the previous configuration. The error appears in Settings and the next time the panel is shown; a reload never opens the panel itself. Fixing the file clears the error. An empty `hotkey` disables the global shortcut.

Set `dictionary_keywords` to a list of aliases for Dictionary search. The defaults are `define` and `d`; add, remove or replace entries to suit your workflow. Valid edits take effect without relaunching.

Aliases must be non-empty, have no leading or trailing whitespace and leave other commands' keywords, such as `in`, available. Dupin reports invalid or conflicting aliases in the panel and ignores those entries; the other aliases still work.

After editing the file, choose **Relaunch** in Settings → General to restart Dupin and load the new configuration. This action remains available when the configuration is read-only, including Nix-managed files. If an update is already staged, relaunch installs it first.

## Updates

A release build (the Homebrew cask or a zip from GitHub Releases) checks its channel every six hours and installs a newer build by itself: stable follows tagged releases, nightly follows the `nightly` prerelease. It downloads the update into a hidden `.Dupin-update-<build>` folder beside the app and keeps it only when it is signed with the same Developer ID as the running app and Gatekeeper accepts it. The installed app stays as it is until Dupin restarts into the update, which happens when the panel hides with Settings closed and nothing it opened still starting, or until you quit with ⌘Q, which installs it for the next launch. Either way the two apps swap places in one step, so the app is never missing; the next launch removes the old one. `~/Library/Logs/Dupin/dupin.log` records each update and each failed check.

Settings' Updates page (⌘, in the panel) shows the running version and what the updater is doing, and its switch sets `auto_update`: off stops the six-hourly checks. Check for Updates checks at once, even with `auto_update = false`; Restart to Update installs a downloaded update now.

When Dupin can't replace its own app (its folder isn't writable by you, or macOS runs it from a translocated copy because it wasn't moved out of Downloads), it only checks: Settings says "Download only" and offers a Download link to the newer build instead of installing it. Move Dupin to `/Applications` or `~/Applications` to let it update itself.

A build from Nix or `cargo run` never updates itself and has no Updates page in Settings; update it through Nix. Set `DUPIN_DISABLE_UPDATES=1` in Dupin's environment to turn the updater off for a launch.

## Nix

The flake's home-manager module installs Dupin and writes `config.toml` from `settings`:

```nix
{
  imports = [ inputs.dupin.homeManagerModules.default ];

  programs.dupin = {
    enable = true;
    settings = {
      hotkey = "alt+space";
      position = "top";
    };
    launchd.enable = true; # start at login
  };
}
```

With `settings` set, `config.toml` is a read-only link into the Nix store. Change it in Nix and activate the new generation; Dupin follows the updated link automatically. Leave `settings` empty to keep the file out of Nix and editable from Settings.

## Input source

Like Alfred's "Force Keyboard", Dupin switches the keyboard input source while the panel shows and switches back when it hides, so a Japanese or other input method doesn't hold the query in composition. `"ascii"` leaves a source that already types ASCII alone and otherwise picks the most recently used one that does (ABC, or an input method's Roman mode); an identifier picks that source; `"keep"` leaves the input source alone.

## Backdrop

A shader can animate behind the panel, the query field and its results, while it shows, as Ghostty's `custom-shader` does behind a terminal. `"flow"` is the built-in one, the accent color flowing as warped noise, swelling as you type. `"off"`, the default, draws nothing and leaves the panel plain glass. Anything else is the path to a WGSL file: `~/` is the home directory, and a relative path is relative to `config.toml`.

Settings offers `flow`, `off`, the shaders in [`examples/shaders`](../examples/shaders) and every `.wgsl` file in the `shaders` folder beside `config.toml`; the Shaders folder row on the Appearance page opens it. Picking an example copies it into the folder, unless a copy is already there, and points `backdrop` at the copy, so you can edit it.

Dupin reads the file each time the panel shows and compiles it when its text has changed, so you can edit a shader and see it at the next hotkey press. A file that can't be read or doesn't compile shows its error in the panel at every summon until it's fixed; meanwhile Dupin keeps the last version that compiled, or draws `flow`. [`examples/shaders`](../examples/shaders) describes how to write one.

The backdrop holds still under System Settings → Accessibility → Display → Reduce motion.

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
