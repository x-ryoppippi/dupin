# Calendar

Enable **Calendar search** in Settings → Search. Type `cal` followed by Space for up to 30 events over the next seven days. To search from 30 days ago to 90 days ahead, type words from the title or location in any order. Each row shows the event time in the Mac's local time zone and its calendar.

Events are grouped under a heading for their day: Today, Tomorrow, then the weekday and date. An event already under way sits under Today. A typed search lists events that have ended last, under Earlier. Under a day's heading, a row shows only the time, or “All day”; an event that spans more than one day, or is listed under Earlier, also shows its dates, with the year when it is not this year. ↑ and ↓ move between events and skip the headings.

Dupin requests Full Access to Calendars when you enable Calendar search in Settings, or on first use if you enabled it in `config.toml`. You can also request it in Settings → Permissions. If access is denied, the permission row opens System Settings → Privacy & Security → Calendars. Invoke `cal` again after granting access there. Calendar results stay out of Snapshots and search history.

Press Enter to open the original event in Calendar. Dupin uses an undocumented Calendar link. If Calendar cannot resolve an event or recurring occurrence, it may open without selecting it.

## Configuration

Turn on **Calendar search** in Settings → Search, or set `enabled = true` in `[calendar]`. This reserves the `cal` keyword. Turning the setting off removes the command immediately.
