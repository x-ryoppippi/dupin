# Calendar

Enable **Calendar search** in Settings → Search. Type `cal` followed by Space for up to 30 events over the next seven days. To search from 30 days ago to 90 days ahead, type words from the title or location in any order. Each row shows the calendar and the event time in the Mac's local time zone. All-day events show their dates and an “All day” label.

The first search may ask for Full Access to Calendars. If access is missing, select the permission row to open System Settings → Privacy & Security → Calendars. Invoke `cal` again after granting access there. Calendar results stay out of Snapshots and search history.

Press Enter to open the original event in Calendar. Dupin uses an undocumented Calendar link. If Calendar cannot resolve an event or recurring occurrence, it may open without selecting it.

## Configuration

Turn on **Calendar search** in Settings → Search, or set `enabled = true` in `[calendar]`. This reserves the `cal` keyword. Dupin asks for Calendar access when you first use the command in the packaged app. Turning the setting off removes the command immediately.
