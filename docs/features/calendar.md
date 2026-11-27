# Calendar

Dupin lists and searches your calendar events.

## Open it

`cal` <kbd>Space</kbd>, or an [alias](./keywords.md#aliases) for `calendar`, once **Calendar search** is on.

## What you see

- With nothing typed, up to 30 events over the next seven days.
- With words typed, in any order, the events whose title or location match, from 30 days ago to 90 days ahead.
- Events under a heading for their day: Today, Tomorrow, then the weekday and date; a typed search lists events that have ended last, under Earlier.
- Each event's time in the Mac's local time zone, and its calendar.

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/calendar/upcoming-dark.png" />
<img alt="Calendar search before anything is typed: events under Today, Tomorrow and a weekday heading, each with its time and calendar." src="../images/calendar/upcoming-light.png" width="680" />
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../images/calendar/clip-dark.webp" />
<img alt="Typing cal and Space enters Calendar search, listing upcoming events by day; ↓ moves between events and skips the headings." src="../images/calendar/clip-light.webp" width="680" />
</picture>

## What you can do

| Key / action              | What it does                                      |
| ------------------------- | ------------------------------------------------- |
| <kbd>↑</kbd> <kbd>↓</kbd> | Move between events, skipping the headings        |
| <kbd>↩</kbd>              | Open the event in Calendar (**Open in Calendar**) |

Other keys: [Keys](../usage.md#keys).

## Notes

- Off by default.
- Needs Full Access to Calendars. Dupin requests it when you enable Calendar search in Settings, or on first use if you enabled it in `config.toml`; you can also request it in Settings → Permissions. If access is denied, the panel and the permission row open System Settings → Privacy & Security → Calendars; invoke `cal` again after granting access there.
- Calendar results stay out of Snapshots and search history.
- Dupin uses an undocumented Calendar link. If Calendar cannot resolve an event or recurring occurrence, it may open without selecting it.

## Configuration

```toml
[calendar]
enabled = true # reserves the cal keyword
```

| Key                  | Default | Meaning                                                                                                                                          |
| -------------------- | ------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `[calendar] enabled` | `false` | Turns Calendar search on and reserves the `cal` keyword; turning it off removes the command immediately. Settings → Search → **Calendar search** |

Calendar has a second switch: its row in Settings → Extensions, which turns the `calendar` package on or off as it does any extension. `cal` answers only while both are on.

<details>
<summary>The two switches</summary>

**Calendar search** decides whether the command and its keyword exist; the package switch decides whether its extension may run. With Calendar search on and the package off, `cal` says Calendar search isn't available, and the package's row says it is off.

</details>

<details>
<summary>How rows show time</summary>

An event already under way sits under Today. Under a day's heading, a row shows only the time, or “All day”; an event that spans more than one day, or is listed under Earlier, also shows its dates, with the year when it is not this year.

</details>

## How it works

<details>
<summary>The bundled extension</summary>

Calendar search is the [bundled extension](./extensions.md#bundled-extensions) `calendar`, a TypeScript package in [`extensions/calendar`](../../extensions/calendar) that ships inside Dupin.app. It runs only while **Calendar search** is on, under the `cal` keyword. Settings → Extensions lists it as a built-in command, with where it comes from, its package switch and its grant, as [Configuration](#configuration) describes. It reads events through the [`calendar` capability](./extension-permissions.md#calendar), which the bundled package holds from the start for 30 days back and 90 ahead, and lays them out itself. macOS's Calendars access is still Dupin's to ask for, as above, never the extension's.

The calendar service matches the typed words against each event's title and location, so a keystroke sends only the matching events to the extension. JavaScript in the extension host has no `Intl`, so the service also hands over each event's day, time and dates as this Mac's locale and time zone write them, and the Calendar link <kbd>↩</kbd> opens.

</details>

<details>
<summary>Changing it</summary>

To change how events are listed, copy the package into the extensions folder and edit it:

```sh
dupin ext eject calendar
```

While the extensions folder is on, your copy answers `cal` in place of the bundled one, still behind **Calendar search**, as [Overriding a built-in](./extensions.md#overriding-a-built-in) describes. Once it differs from the bundled package it is a third-party extension: the panel asks you to turn it on and to allow it to read calendars before it shows any events. Delete the copy to go back. `dupin ext test` replays the package's [fixtures](./extension-testing.md#fixtures) in `extensions/calendar/fixtures`.

</details>
