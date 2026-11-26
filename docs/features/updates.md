# Updates

A release build (the Homebrew cask or a zip from GitHub Releases) checks its channel every six hours and installs a newer build by itself.

## Open it

Open **Settings → Updates** (<kbd>⌘,</kbd> in the panel). A build from Nix or `cargo run` has no Updates page.

## What you see

- The running version.
- What the updater is doing: **Checking…**, **Downloading…**, **Update ready** or **Update available**.
- The **Install updates automatically** switch.

## What you can do

| Key / action                         | What it does                                                                                                                          |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------- |
| **Install updates automatically**    | Sets [`auto_update`](#configuration); off stops the six-hourly checks                                                                 |
| **Check for Updates**                | Checks at once, even with `auto_update = false`                                                                                       |
| **Restart to Update**                | Installs a downloaded update now                                                                                                      |
| **Download**                         | In "Download only" mode, opens the newer build's download instead of installing it                                                    |
| <kbd>⌘Q</kbd> in the panel           | Quits; a downloaded update installs for the next launch                                                                               |
| **Update Dupin** in the main search  | Restarts into a downloaded update, or checks now and reports the outcome in the launcher; see [System commands](./system-commands.md) |
| **Restart Dupin** in the main search | Relaunches Dupin                                                                                                                      |

## Notes

- Stable follows tagged releases; nightly follows the `nightly` prerelease.
- Without **Restart to Update**, Dupin restarts into a downloaded update when the panel hides with Settings closed and nothing it opened still starting.
- If Settings says "Download only", move Dupin to `/Applications` or `~/Applications` to let it update itself.
- `~/Library/Logs/Dupin/dupin.log` records each update and each failed check.
- A build from Nix or `cargo run` never updates itself; update it through Nix. Set `DUPIN_DISABLE_UPDATES=1` in Dupin's environment to turn the updater off for a launch.

## Configuration

```toml
auto_update = true # whether a release build checks for and installs its channel's updates by itself
```

| Key           | Default | Meaning                                                                         |
| ------------- | ------- | ------------------------------------------------------------------------------- |
| `auto_update` | `true`  | Whether a release build checks for and installs its channel's updates by itself |

<details>
<summary>How it installs</summary>

Dupin downloads the update into a hidden `.Dupin-update-` folder of its own beside the app and keeps it only when it is signed with the same Developer ID as the running app and Gatekeeper accepts it. The installed app stays as it is until Dupin restarts into the update, which happens when the panel hides with Settings closed and nothing it opened still starting, or until you quit with ⌘Q, which installs it for the next launch. Either way the two apps swap places in one step, so the app is never missing; the next launch removes the old one.

</details>

<details>
<summary>Download only</summary>

When Dupin can't replace its own app (its folder isn't writable by you, or macOS runs it from a translocated copy because it wasn't moved out of Downloads), it only checks: Settings says "Download only" and offers a Download link to the newer build instead of installing it.

</details>
