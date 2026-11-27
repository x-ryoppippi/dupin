# Calendar

Lists upcoming calendar events, and searches events by title or location.

| Keyword       | `cal`                                                       |
| ------------- | ----------------------------------------------------------- |
| Command       | `calendar`                                                  |
| On by default | No: it waits for **Calendar search** (`[calendar] enabled`) |
| User docs     | [Calendar](../../docs/features/calendar.md)                 |

A port of the former native Calendar search; see [Built-ins as bundled extensions](../../docs/design/extension-platform.md#built-ins-as-bundled-extensions). It answers the built-in command under that command's keyword.

## How it's built

- **`search(query, ctx)`** with **`emptyQuery: true`**, so it answers as soon as the keyword is typed: the next seven days with nothing typed, or matches from 30 days back to 90 ahead. It returns a `List` of `sections`, one per day (Today, Tomorrow, then the day's label) and Earlier for events that have ended.
- **Actions:** **Open in Calendar** opens the event's link.
- **Capabilities:** `calendar: { access: ["read"], pastDays: 30, futureDays: 90 }`, for `calendar.events(...)` over exactly the range the search covers. macOS Calendars access is asked for separately, only by **Request Access**.

## Files

| Path              | What                                                                       |
| ----------------- | -------------------------------------------------------------------------- |
| `package.json`    | The manifest.                                                              |
| `main.ts`         | `search`, the day headings and each event's row.                           |
| `fixtures/*.json` | Upcoming events with nothing typed, a typed search, consent still pending. |

## Test it

```sh
dupin ext test extensions/calendar
dupin ext render extensions/calendar extensions/calendar/fixtures/upcoming.json upcoming.png
```

Each fixture lists the events a read can see in `replies.calendar`, and `consent.json` sets the scope to `pending` with `replies.consent`. The fixed `environment.now` decides which day is Today. Run `dupin` from this checkout's build; see [Working on them](../README.md#working-on-them).
