# Telemetry

Official Dupin releases send one small anonymous usage ping a day, so the project can see how many people use it, on which macOS versions, and how quickly a release is picked up. It is on by default after Dupin tells you about it, never sends anything you search, open or set, and turns off with one switch.

## Fully transparent

Every part of it is public, so you never have to take this page's word for it:

- **Client source**: what decides whether, when and what to send is [`crates/dupin/src/telemetry.rs`](https://github.com/x-ryoppippi/dupin/blob/main/crates/dupin/src/telemetry.rs).
- **Server source**: the collector is a small Cloudflare Worker in [`telemetry/`](https://github.com/x-ryoppippi/dupin/tree/main/telemetry), in the same repository, so you can read exactly what it validates and stores.
- **Public statistics**: the [usage dashboard](https://telemetry.dupin.app/) publishes the daily aggregates, and so does its [JSON endpoint](https://telemetry.dupin.app/v1/stats). Neither exposes an anonymous ID.
- **One definition of the payload**: the ping is the Rust [`Ping`](https://github.com/x-ryoppippi/dupin/blob/main/crates/dupin-telemetry/src/lib.rs) type. The collector's validation schema is generated from it, and CI fails if the two disagree, so Dupin can't quietly start sending a field the collector doesn't document.
- **Builds**: releases are built, signed and notarized by the public [package workflow](https://github.com/x-ryoppippi/dupin/blob/main/.github/workflows/package.yml) on GitHub Actions, never on a developer's Mac.

## What is sent

At most once per UTC day, an official release sends one JSON document to `https://telemetry.dupin.app/v1/ping`:

```json
{
	"event": "dupin_daily_active",
	"anonymousId": "4fd4c40cb4140e0e8952ceb2b3502e07",
	"version": "0.4.0",
	"channel": "stable",
	"os": "macos 26",
	"arch": "aarch64"
}
```

| Field         | Meaning                                                                                                                                 |
| ------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| `event`       | Always `dupin_daily_active`                                                                                                             |
| `anonymousId` | A random 128-bit value generated on your Mac, not derived from its hardware or anything else, kept in [encrypted storage](./storage.md) |
| `version`     | Dupin's version                                                                                                                         |
| `channel`     | `stable` or `nightly`                                                                                                                   |
| `os`          | `macos` and the major macOS version                                                                                                     |
| `arch`        | The CPU architecture                                                                                                                    |

That is the complete list. Dupin never sends what you type or search, the apps, files or results you open, your extensions, snippets, clipboard, settings or hotkeys, or any path or name from your Mac.

The ping leaves from a background thread, never from the panel or a keystroke, with a ten-second limit. A slow or failed request is dropped and not retried; the next one is the next day's.

## What is stored

The collector writes only the six fields above to its Cloudflare Analytics Engine dataset. It does not write IP addresses, headers or request metadata, and Workers Logs are off. Cloudflare handles the connection and therefore processes the source IP as part of operating its network. The persistent random ID lets the same installation be counted across days: this is pseudonymous measurement, not untraceable data.

- **Individual pings** expire after Analytics Engine's three-month retention. Maintainers with account access can query them.
- **Daily totals** are computed once a day and kept: the number of distinct installations that day and over the last 7 and 30 days, and installations per platform, version and channel. They contain no anonymous IDs.

## Public dashboard

The [dashboard](https://telemetry.dupin.app/) shows up to 90 completed UTC days of daily, weekly and monthly active installations, and the platform, version and channel split. It reads the stored daily totals only: viewing it neither reads individual pings nor counts you. It loads no third-party scripts and no page analytics.

An active installation is a distinct random ID seen in the window, not a verified person: several Macs, or turning telemetry off and on again, count more than once, and opt-outs, blocked requests and failed sends count less. Breakdowns keep the 20 largest groups per day. Totals are generated after 00:15 UTC; missing days show as gaps.

## First-run notice

Nothing is sent before Dupin has told you:

- On a first run, the welcome in Settings has an **Anonymous usage ping** row that says what is sent, with **Details** (this page) and a switch to turn it off.
- If you used Dupin before telemetry existed, the panel shows a one-time notice the next time it opens.

The launch that told you sends nothing, so you can turn it off before the first ping; a later launch sends at most one ping a day.

## Opting out

Any of these turns it off, and each takes effect at once, in the running launcher:

- **Settings → General → Anonymous usage ping**: switch it off.
- In `config.toml`:

  ```toml
  telemetry = false
  ```

  With Home Manager, `programs.dupin.settings.telemetry = false;` sets it, and Settings shows the row as managed by Nix.

- From a terminal, with Dupin running:

  ```sh
  dupin telemetry disable
  dupin telemetry enable
  dupin telemetry status # or just `dupin telemetry`; add --json for JSON
  ```

  These change the same `telemetry` key as Settings, and `status` says whether this Dupin sends the ping and, if not, why.

- In Dupin's environment, `DUPIN_TELEMETRY_DISABLED=1` or `DO_NOT_TRACK=1`; any value but empty, `0` or `false` counts. Settings then shows the switch off and says which variable turned it off.

Turning the setting off deletes the stored anonymous ID, so turning it on again later starts as a new installation. If the stored state can't be read, nothing is sent. `telemetry` is a per-Mac setting: [Sync](./sync.md) never carries it.

## Builds that never send

Only the build that the release workflow signs with Dupin's Developer ID carries the collector's address, compiled in by `nix build .#dupin-release`. These contain no address and never send anything:

- The Nix package (`nix build .#dupin`, `nix profile install`, the Home Manager module).
- `cargo run` and `cargo build`.
- A copy you package yourself with `just package` and an ad-hoc signature: it has the address, but Dupin also checks for a Developer ID signature, as the [updater](./updates.md) does.

Settings shows such a build's row with "Never sent from this build".

## Configuration

| Key         | Default | Meaning                                                                                 |
| ----------- | ------- | --------------------------------------------------------------------------------------- |
| `telemetry` | `true`  | Whether an official release sends the anonymous daily usage ping; other builds never do |

| Variable                   | Effect                                              |
| -------------------------- | --------------------------------------------------- |
| `DUPIN_TELEMETRY_DISABLED` | Any value except empty, `0` or `false` turns it off |
| `DO_NOT_TRACK`             | The same, following the `DO_NOT_TRACK` convention   |
