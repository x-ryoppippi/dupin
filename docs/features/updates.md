# Updates

A release build (the Homebrew cask or a zip from GitHub Releases) checks its channel every six hours and installs a newer build by itself: stable follows tagged releases, nightly follows the `nightly` prerelease. It downloads the update into a hidden `.Dupin-update-` folder of its own beside the app and keeps it only when it is signed with the same Developer ID as the running app and Gatekeeper accepts it. The installed app stays as it is until Dupin restarts into the update, which happens when the panel hides with Settings closed and nothing it opened still starting, or until you quit with ⌘Q, which installs it for the next launch. Either way the two apps swap places in one step, so the app is never missing; the next launch removes the old one. `~/Library/Logs/Dupin/dupin.log` records each update and each failed check.

Settings' Updates page (⌘, in the panel) shows the running version and what the updater is doing, and its switch sets `auto_update`: off stops the six-hourly checks. Check for Updates checks at once, even with `auto_update = false`; Restart to Update installs a downloaded update now.

When Dupin can't replace its own app (its folder isn't writable by you, or macOS runs it from a translocated copy because it wasn't moved out of Downloads), it only checks: Settings says "Download only" and offers a Download link to the newer build instead of installing it. Move Dupin to `/Applications` or `~/Applications` to let it update itself.

A build from Nix or `cargo run` never updates itself and has no Updates page in Settings; update it through Nix. Set `DUPIN_DISABLE_UPDATES=1` in Dupin's environment to turn the updater off for a launch.

## Configuration

```toml
auto_update = true # whether a release build checks for and installs its channel's updates by itself
```
