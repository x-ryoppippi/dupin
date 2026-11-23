use dupin_core::{Action, Item, ItemAction, Provider, ProviderError, Request, View};
use objc2::rc::{Retained, autoreleasepool};
use objc2::{Message, msg_send};
use objc2_event_kit::{EKAuthorizationStatus, EKEvent, EKEventStatus, EKEventStore};
use objc2_foundation::{
    NSArray, NSCalendar, NSCalendarOptions, NSCalendarUnit, NSDate, NSDateFormatter, NSDateIntervalFormatter,
    NSDateIntervalFormatterStyle, NSLocale, NSString, NSTimeZone,
};

use crate::privacy::{Access, CalendarAccess, Permission, PermissionStatus, calendar_authorization};

const FIXTURE_NOW: f64 = 1_795_420_800.0;

thread_local! {
    static STORE: Retained<EKEventStore> = unsafe { EKEventStore::new() };
}

enum Source {
    EventKit(CalendarAccess),
    Fixtures,
}

pub struct CalendarProvider(Source);

impl CalendarProvider {
    pub fn new(access: CalendarAccess) -> Self {
        Self(Source::EventKit(access))
    }

    pub fn fixtures() -> Self {
        Self(Source::Fixtures)
    }
}

impl Provider for CalendarProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        if request.limit == 0 || request.is_cancelled() {
            return Ok(());
        }
        autoreleasepool(|_| {
            let (now, zone) = match &self.0 {
                Source::EventKit(access) => {
                    if request.is_cancelled() {
                        return Ok(());
                    }
                    if let Some(row) = access_problem(access.request_on_use()) {
                        emit(vec![row]);
                        return Ok(());
                    }
                    (NSDate::date(), NSTimeZone::systemTimeZone())
                }
                Source::Fixtures => (
                    NSDate::dateWithTimeIntervalSince1970(FIXTURE_NOW),
                    NSTimeZone::timeZoneForSecondsFromGMT(0),
                ),
            };
            let calendar = NSCalendar::currentCalendar();
            calendar.setTimeZone(&zone);
            let query = request.query.to_lowercase();
            let terms: Vec<_> = query.split_whitespace().collect();
            let (start, end) = bounds(&calendar, &now, terms.is_empty()).ok_or(ProviderError)?;
            let events = match &self.0 {
                Source::EventKit(_) => {
                    let events = STORE.with(|store| read_events(store, &start, &end, request));
                    if calendar_authorization() != EKAuthorizationStatus::FullAccess {
                        emit(vec![permission_row()]);
                        return Ok(());
                    }
                    events
                }
                Source::Fixtures => fixture_events(&now),
            };
            if !request.is_cancelled() {
                emit(event_rows(
                    events,
                    &terms,
                    request.limit,
                    &now,
                    &start,
                    &end,
                    &zone,
                ));
            }
            Ok(())
        })
    }
}

fn access_problem(status: PermissionStatus) -> Option<Item> {
    Some(match status {
        PermissionStatus::Access(Access::Allowed) => return None,
        PermissionStatus::Access(_) => permission_row(),
        PermissionStatus::Unavailable => row(
            "app-bundle",
            "Calendar access is unavailable",
            "Open the packaged Dupin.app to grant access.".into(),
            None,
        ),
        PermissionStatus::Requesting => row(
            "permission-pending",
            "Waiting for Calendar access",
            "Respond to the macOS permission request.".into(),
            None,
        ),
        PermissionStatus::RequestIncomplete => row(
            "permission-incomplete",
            "Calendar access request did not complete",
            "Try Request Access in Dupin Settings → Permissions.".into(),
            None,
        ),
    })
}

fn permission_row() -> Item {
    row(
        "permission",
        "Calendar access required",
        "Allow Full Access in System Settings → Privacy & Security → Calendars.".into(),
        Some((
            "Open Calendar Privacy Settings",
            Permission::Calendars.settings_url().into(),
        )),
    )
}

fn bounds(
    calendar: &NSCalendar,
    now: &NSDate,
    upcoming: bool,
) -> Option<(Retained<NSDate>, Retained<NSDate>)> {
    let offset = |days| {
        calendar.dateByAddingUnit_value_toDate_options(
            NSCalendarUnit::Day,
            days,
            now,
            NSCalendarOptions::empty(),
        )
    };
    Some(if upcoming {
        (now.retain(), offset(7)?)
    } else {
        (offset(-30)?, offset(90)?)
    })
}

struct Event {
    id: String,
    title: String,
    location: String,
    calendar: String,
    start: Retained<NSDate>,
    end: Retained<NSDate>,
    all_day: bool,
    occurrence: Option<Retained<NSDate>>,
}

fn read_events(store: &EKEventStore, start: &NSDate, end: &NSDate, request: &Request) -> Vec<Event> {
    let events: Option<Retained<NSArray<EKEvent>>> = unsafe {
        let predicate = store.predicateForEventsWithStartDate_endDate_calendars(start, end, None);
        msg_send![store, eventsMatchingPredicate: &*predicate]
    };
    events
        .iter()
        .flat_map(|events| events.iter())
        .take_while(|_| !request.is_cancelled())
        .filter_map(|event| unsafe {
            if event.status() == EKEventStatus::Canceled {
                return None;
            }
            let start: Option<Retained<NSDate>> = msg_send![&*event, startDate];
            let end: Option<Retained<NSDate>> = msg_send![&*event, endDate];
            let title: Option<Retained<NSString>> = msg_send![&*event, title];
            Some(Event {
                id: event.calendarItemIdentifier().to_string(),
                title: title.map_or_else(String::new, |title| title.to_string()),
                location: event
                    .location()
                    .map_or_else(String::new, |location| location.to_string()),
                calendar: event
                    .calendar()
                    .map_or_else(|| "Calendar".into(), |calendar| calendar.title().to_string()),
                start: start?,
                end: end?,
                all_day: event.isAllDay(),
                occurrence: event.occurrenceDate(),
            })
        })
        .collect()
}

fn event_rows(
    mut events: Vec<Event>,
    terms: &[&str],
    limit: usize,
    now: &NSDate,
    start: &NSDate,
    end: &NSDate,
    zone: &NSTimeZone,
) -> Vec<Item> {
    let now = now.timeIntervalSince1970();
    let start = start.timeIntervalSince1970();
    let end = end.timeIntervalSince1970();
    events.retain(|event| {
        let begins = event.start.timeIntervalSince1970();
        if begins >= end || (begins < start && event.end.timeIntervalSince1970() <= start) {
            return false;
        }
        if terms.is_empty() {
            return true;
        }
        let title = event.title.to_lowercase();
        let location = event.location.to_lowercase();
        terms
            .iter()
            .all(|term| title.contains(term) || location.contains(term))
    });
    let chronological_key = |event: &Event| {
        let start = event.start.timeIntervalSince1970();
        let past = start < now && event.end.timeIntervalSince1970() <= now;
        (past, if past { -start } else { start })
    };
    events.sort_by(|a, b| {
        let (a_past, a_start) = chronological_key(a);
        let (b_past, b_start) = chronological_key(b);
        a_past
            .cmp(&b_past)
            .then_with(|| a_start.total_cmp(&b_start))
            .then_with(|| a.id.cmp(&b.id))
    });
    let formatter = NSDateIntervalFormatter::new();
    formatter.setDateStyle(NSDateIntervalFormatterStyle::MediumStyle);
    formatter.setTimeZone(Some(zone));
    events
        .into_iter()
        .take(limit)
        .map(|event| {
            let occurrence = event.occurrence.as_ref().unwrap_or(&event.start);
            let id = format!("{}:{}", event.id, occurrence.timeIntervalSince1970());
            let url = event_url(&event, zone);
            let subtitle = format!("{} · {}", event_time(&event, &formatter), event.calendar);
            let title = if event.title.trim().is_empty() {
                "Untitled event"
            } else {
                &event.title
            };
            row(&id, title, subtitle, Some(("Open in Calendar", url)))
        })
        .collect()
}

fn event_time(event: &Event, formatter: &NSDateIntervalFormatter) -> String {
    let start = event.start.timeIntervalSince1970();
    let end = event.end.timeIntervalSince1970();
    if event.all_day {
        formatter.setTimeStyle(NSDateIntervalFormatterStyle::NoStyle);
        // All-day events end at the start of the next day.
        let inclusive_end = NSDate::dateWithTimeIntervalSince1970((end - 1.0).max(start));
        format!(
            "{} · All day",
            formatter.stringFromDate_toDate(&event.start, &inclusive_end)
        )
    } else {
        formatter.setTimeStyle(NSDateIntervalFormatterStyle::ShortStyle);
        let end = if end < start { &event.start } else { &event.end };
        formatter.stringFromDate_toDate(&event.start, end).to_string()
    }
}

fn event_url(event: &Event, zone: &NSTimeZone) -> String {
    let id = crate::workspace::encode_url_component(&event.id);
    let occurrence = event.occurrence.as_ref().map_or_else(String::new, |date| {
        let formatter = NSDateFormatter::new();
        formatter.setLocale(Some(&NSLocale::localeWithLocaleIdentifier(&NSString::from_str(
            "en_US_POSIX",
        ))));
        let utc = NSTimeZone::timeZoneForSecondsFromGMT(0);
        // Calendar uses a local date stamp for floating all-day occurrences.
        formatter.setTimeZone(Some(if event.all_day { zone } else { &utc }));
        formatter.setDateFormat(Some(&NSString::from_str("yyyyMMdd'T'HHmmss'Z'")));
        format!("{}/", formatter.stringFromDate(date))
    });
    format!("ical://ekevent/{occurrence}{id}?method=show&options=more")
}

fn row(id: &str, title: &str, subtitle: String, open: Option<(&str, String)>) -> Item {
    Item {
        payload: Default::default(),
        id: id.into(),
        view: View::Text {
            title: title.into(),
            subtitle,
        },
        record_usage: false,
        icon: None,
        matched: Vec::new(),
        completion: None,
        score: None,
        primary: open.as_ref().map(|_| "open".into()),
        alternate: None,
        actions: open
            .map(|(label, url)| ItemAction::new("open", label, Action::OpenUrl(url)))
            .into_iter()
            .collect(),
    }
}

fn fixture_events(now: &NSDate) -> Vec<Event> {
    [
        ("Project planning", "Studio", "Work", 7_200, 10_800, false),
        ("Design review", "Meeting room", "Work", 28_800, 30_600, false),
        ("Out of office", "", "Personal", 57_600, 144_000, true),
    ]
    .into_iter()
    .map(|(title, location, calendar, start, end, all_day)| Event {
        id: title.replace(' ', "-").to_lowercase(),
        title: title.into(),
        location: location.into(),
        calendar: calendar.into(),
        start: now.dateByAddingTimeInterval(f64::from(start)),
        end: now.dateByAddingTimeInterval(f64::from(end)),
        all_day,
        occurrence: None,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(query: &str, limit: usize) -> Vec<Item> {
        let mut rows = Vec::new();
        CalendarProvider::fixtures()
            .search(&Request::detached(query, limit), &mut |batch| rows.extend(batch))
            .unwrap();
        rows
    }

    #[test]
    fn fixtures_list_upcoming_in_order_and_filter_by_title_or_location() {
        assert_eq!(
            collect("", 2).iter().map(Item::display_title).collect::<Vec<_>>(),
            ["Project planning", "Design review"],
        );
        for query in [" DESIGN ", "meeting ROOM", "review design", "room DESIGN"] {
            let rows = collect(query, 30);
            assert_eq!(rows.len(), 1, "{query}");
            assert_eq!(rows[0].display_title(), "Design review");
            assert!(!rows[0].record_usage);
        }
        assert!(collect("Work", 30).is_empty());
        assert!(collect("", 0).is_empty());
    }

    #[test]
    fn search_windows_include_ongoing_events_and_exclude_out_of_range_events() {
        let now = NSDate::dateWithTimeIntervalSince1970(FIXTURE_NOW);
        let zone = NSTimeZone::timeZoneForSecondsFromGMT(0);
        let calendar = NSCalendar::currentCalendar();
        calendar.setTimeZone(&zone);
        let (start, end) = bounds(&calendar, &now, true).unwrap();
        let mut events = fixture_events(&now);
        events[0].start = now.dateByAddingTimeInterval(-3600.0);
        events[1].start = end.clone();
        events[1].end = end.dateByAddingTimeInterval(3600.0);
        events[2].start = now.dateByAddingTimeInterval(-86_400.0);
        events[2].end = now.clone();
        let rows = event_rows(events, &[], 30, &now, &start, &end, &zone);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].display_title(), "Project planning");
        let (start, end) = bounds(&calendar, &now, false).unwrap();
        assert_eq!(now.timeIntervalSinceDate(&start), 30.0 * 86_400.0);
        assert_eq!(end.timeIntervalSinceDate(&now), 90.0 * 86_400.0);
    }

    #[test]
    fn typed_search_keeps_ongoing_and_upcoming_occurrences_ahead_of_past_ones() {
        let now = NSDate::dateWithTimeIntervalSince1970(FIXTURE_NOW);
        let zone = NSTimeZone::timeZoneForSecondsFromGMT(0);
        let start = now.dateByAddingTimeInterval(-30.0 * 86_400.0);
        let end = now.dateByAddingTimeInterval(90.0 * 86_400.0);
        let events = [-7200.0, 7200.0, -3600.0, -900.0, 3600.0]
            .into_iter()
            .map(|offset| {
                let start = now.dateByAddingTimeInterval(offset);
                Event {
                    id: "standup".into(),
                    title: "Standup".into(),
                    location: String::new(),
                    calendar: "Work".into(),
                    occurrence: Some(start.clone()),
                    end: start.dateByAddingTimeInterval(1800.0),
                    start,
                    all_day: false,
                }
            })
            .collect();
        let rows = event_rows(events, &["standup"], 4, &now, &start, &end, &zone);
        let expected =
            [-900.0, 3600.0, 7200.0, -3600.0].map(|offset| format!("standup:{}", FIXTURE_NOW + offset));
        assert_eq!(rows.into_iter().map(|row| row.id).collect::<Vec<_>>(), expected);
    }

    #[test]
    fn date_ranges_use_the_output_zone_and_exclude_the_all_day_end_date() {
        let now = NSDate::dateWithTimeIntervalSince1970(FIXTURE_NOW);
        let mut events = fixture_events(&now);
        let formatter = NSDateIntervalFormatter::new();
        formatter.setLocale(Some(&NSLocale::localeWithLocaleIdentifier(&NSString::from_str(
            "en_GB",
        ))));
        formatter.setDateStyle(NSDateIntervalFormatterStyle::MediumStyle);
        formatter.setTimeZone(Some(&NSTimeZone::timeZoneForSecondsFromGMT(-8 * 3600)));
        let timed = event_time(&events[0], &formatter);
        assert!(timed.contains("02:00") && timed.contains("03:00"), "{timed}");

        formatter.setTimeZone(Some(&NSTimeZone::timeZoneForSecondsFromGMT(0)));
        let all_day = event_time(&events[2], &formatter);
        assert!(all_day.contains("24 Nov") && !all_day.contains("25"), "{all_day}");
        assert!(all_day.ends_with("All day"));
        events[2].end = events[2].end.dateByAddingTimeInterval(86_400.0);
        assert!(event_time(&events[2], &formatter).contains("25"));
    }

    #[test]
    fn calendar_links_escape_ids_and_include_recurring_occurrences() {
        let now = NSDate::dateWithTimeIntervalSince1970(FIXTURE_NOW);
        let mut events = fixture_events(&now);
        let event = &mut events[0];
        event.id = "shared/meeting?".into();
        let zone = NSTimeZone::timeZoneForSecondsFromGMT(3600);
        assert_eq!(
            event_url(event, &zone),
            "ical://ekevent/shared%2Fmeeting%3F?method=show&options=more"
        );
        event.occurrence = Some(event.start.clone());
        assert_eq!(
            event_url(event, &zone),
            "ical://ekevent/20261123T100000Z/shared%2Fmeeting%3F?method=show&options=more"
        );
        assert_eq!(
            permission_row().action("open"),
            Some(&Action::OpenUrl(Permission::Calendars.settings_url().into()))
        );
    }
}
