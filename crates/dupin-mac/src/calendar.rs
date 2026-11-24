use dupin_core::{Action, Batch, Item, ItemAction, Provider, ProviderError, Request, View};
use dupin_host_services::calendar::{self as service, CalendarInfo, Calendars, Millis};
use dupin_host_services::{OsAccess, ServiceError};
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2::{Message, class, msg_send};
use objc2_event_kit::{EKAuthorizationStatus, EKCalendar, EKEvent, EKEventStatus, EKEventStore};
use objc2_foundation::{
    NSArray, NSCalendar, NSCalendarOptions, NSCalendarUnit, NSDate, NSDateFormatter, NSDateIntervalFormatter,
    NSLocale, NSString, NSTimeZone, NSURL,
};

use crate::privacy::{Access, CalendarAccess, Permission, PermissionStatus, calendar_authorization};

/// When the fixture events are laid out around, in milliseconds: 23 November 2026, 08:00 UTC.
pub const FIXTURE_CLOCK: i64 = 1_795_420_800_000;
const FIXTURE_NOW: f64 = FIXTURE_CLOCK as f64 / 1000.0;

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
        self.search_batches(request, &mut |batch| emit(batch.items))
    }

    fn search_batches(&self, request: &Request, emit: &mut dyn FnMut(Batch)) -> Result<(), ProviderError> {
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
                        emit(vec![row].into());
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
                    let events =
                        STORE.with(|store| read_events(store, &start, &end, &|| request.is_cancelled()));
                    if calendar_authorization() != EKAuthorizationStatus::FullAccess {
                        emit(vec![permission_row()].into());
                        return Ok(());
                    }
                    events
                }
                Source::Fixtures => fixture_events(&now),
            };
            if !request.is_cancelled() {
                let window = Window {
                    now: &now,
                    start: &start,
                    end: &end,
                    zone: &zone,
                    calendar: &calendar,
                };
                for batch in event_rows(events, &terms, request.limit, &window) {
                    emit(batch);
                }
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
    url: Option<String>,
    color: Option<String>,
}

fn read_events(
    store: &EKEventStore,
    start: &NSDate,
    end: &NSDate,
    cancelled: &dyn Fn() -> bool,
) -> Vec<Event> {
    let events: Option<Retained<NSArray<EKEvent>>> = unsafe {
        let predicate = store.predicateForEventsWithStartDate_endDate_calendars(start, end, None);
        msg_send![store, eventsMatchingPredicate: &*predicate]
    };
    events
        .iter()
        .flat_map(|events| events.iter())
        .take_while(|_| !cancelled())
        .filter_map(|event| unsafe {
            if event.status() == EKEventStatus::Canceled {
                return None;
            }
            let start: Option<Retained<NSDate>> = msg_send![&*event, startDate];
            let end: Option<Retained<NSDate>> = msg_send![&*event, endDate];
            let title: Option<Retained<NSString>> = msg_send![&*event, title];
            let url: Option<Retained<NSURL>> = msg_send![&*event, URL];
            let calendar = event.calendar();
            Some(Event {
                id: event.calendarItemIdentifier().to_string(),
                title: title.map_or_else(String::new, |title| title.to_string()),
                location: event
                    .location()
                    .map_or_else(String::new, |location| location.to_string()),
                color: calendar.as_deref().and_then(color),
                calendar: calendar.map_or_else(|| "Calendar".into(), |calendar| calendar.title().to_string()),
                start: start?,
                end: end?,
                all_day: event.isAllDay(),
                occurrence: event.occurrenceDate(),
                url: url
                    .and_then(|url| url.absoluteString())
                    .map(|url| url.to_string()),
            })
        })
        .collect()
}

/// The calendar's color in sRGB.
fn color(calendar: &EKCalendar) -> Option<String> {
    // SAFETY: `color` and the NSColor methods below take no arguments but the color space and
    // return an object or a CGFloat, as declared.
    unsafe {
        let color: Option<Retained<AnyObject>> = msg_send![calendar, color];
        let space: Option<Retained<AnyObject>> = msg_send![class!(NSColorSpace), sRGBColorSpace];
        let srgb: Option<Retained<AnyObject>> = msg_send![&*color?, colorUsingColorSpace: &*space?];
        let srgb = srgb?;
        let red: f64 = msg_send![&*srgb, redComponent];
        let green: f64 = msg_send![&*srgb, greenComponent];
        let blue: f64 = msg_send![&*srgb, blueComponent];
        Some(service::hex_color(red, green, blue))
    }
}

/// The extension `calendar` capability. It only reads Calendars access, never requests it:
/// the app asks on enable or a deliberate action routed from it.
impl Calendars for CalendarProvider {
    fn access(&self) -> OsAccess {
        match &self.0 {
            Source::EventKit(access) => access.status().os_access(),
            Source::Fixtures => OsAccess::Allowed,
        }
    }

    fn day_offset(&self, at: Millis) -> Result<i64, ServiceError> {
        autoreleasepool(|_| {
            let (now, calendar) = self.clock();
            let day = |date: &NSDate| day_number(&calendar, date) as i64;
            Ok(day(&date(at)) - day(&now))
        })
    }

    fn events(&self, start: Millis, end: Millis) -> Result<Vec<service::Event>, ServiceError> {
        autoreleasepool(|_| {
            let events = match &self.0 {
                Source::EventKit(_) => {
                    let events = STORE.with(|store| read_events(store, &date(start), &date(end), &|| false));
                    // EventKit answers with no events when access was taken away meanwhile.
                    if calendar_authorization() != EKAuthorizationStatus::FullAccess {
                        return Err(ServiceError::PermissionDenied(service::PERMISSION));
                    }
                    events
                }
                Source::Fixtures => fixture_events(&self.clock().0),
            };
            Ok(events.into_iter().map(Event::into_service).collect())
        })
    }

    fn describe(&self, events: &mut [service::Event]) {
        if events.is_empty() {
            return;
        }
        autoreleasepool(|_| {
            let (now, calendar) = self.clock();
            let zone = calendar.timeZone();
            let key = format!(
                "{} {} {}",
                zone.name(),
                NSLocale::currentLocale().localeIdentifier(),
                calendar.calendarIdentifier()
            );
            WRITERS.with_borrow_mut(|kept| {
                let writers = match kept {
                    Some(writers) if writers.key == key => {
                        writers.days.at(&now);
                        writers.times.at(&now);
                        writers
                    }
                    _ => kept.insert(Writers {
                        days: Days::new(&now, &calendar),
                        times: Times::new(&now, &calendar, &zone),
                        links: Links::new(&zone),
                        key,
                    }),
                };
                describe(events, &writers.days, &writers.times, &writers.links);
            });
        })
    }
}

/// What `describe` writes dates and links with, kept for as long as the time zone, locale and
/// calendar stay the same: building a date formatter takes far longer than using it.
struct Writers {
    key: String,
    days: Days,
    times: Times,
    links: Links,
}

thread_local! {
    static WRITERS: std::cell::RefCell<Option<Writers>> = const { std::cell::RefCell::new(None) };
}

fn describe(events: &mut [service::Event], days: &Days, times: &Times, links: &Links) {
    for event in events {
        let local = Event::from_service(event);
        event.link = Some(links.url(&local));
        event.labels = service::Labels {
            day: days.date(&local.start),
            time: times
                .within_one_day(&local)
                .then(|| times.text(&local, Some(day_number(&times.calendar, &local.start)))),
            dates: times.text(&local, None),
        };
    }
}

impl CalendarProvider {
    /// Now and the user's calendar, or the fixtures' fixed time in UTC.
    fn clock(&self) -> (Retained<NSDate>, Retained<NSCalendar>) {
        let calendar = NSCalendar::currentCalendar();
        match self.0 {
            Source::EventKit(_) => (NSDate::date(), calendar),
            Source::Fixtures => {
                calendar.setTimeZone(&NSTimeZone::timeZoneForSecondsFromGMT(0));
                (NSDate::dateWithTimeIntervalSince1970(FIXTURE_NOW), calendar)
            }
        }
    }
}

fn date(at: Millis) -> Retained<NSDate> {
    NSDate::dateWithTimeIntervalSince1970(at as f64 / 1000.0)
}

fn millis(date: &NSDate) -> Millis {
    (date.timeIntervalSince1970() * 1000.0).round() as Millis
}

impl Event {
    /// Its `id` names the EventKit event and, after the last `:`, the occurrence.
    fn into_service(self) -> service::Event {
        let occurrence = self.occurrence.as_ref().unwrap_or(&self.start);
        service::Event {
            link: None,
            labels: service::Labels::default(),
            id: format!("{}:{}", self.id, millis(occurrence)),
            start: millis(&self.start),
            end: millis(&self.end),
            all_day: self.all_day,
            calendar: CalendarInfo {
                name: self.calendar,
                color: self.color,
            },
            location: Some(self.location).filter(|location| !location.is_empty()),
            url: self.url,
            title: self.title,
        }
    }

    /// What dates and links are written from.
    fn from_service(event: &service::Event) -> Self {
        let (id, occurrence) = event
            .id
            .rsplit_once(':')
            .and_then(|(id, at)| Some((id, date(at.parse().ok()?))))
            .map_or((event.id.as_str(), None), |(id, at)| (id, Some(at)));
        Self {
            id: id.into(),
            title: String::new(),
            location: String::new(),
            calendar: String::new(),
            start: date(event.start),
            end: date(event.end),
            all_day: event.all_day,
            occurrence,
            url: None,
            color: None,
        }
    }
}

struct Window<'a> {
    now: &'a NSDate,
    start: &'a NSDate,
    end: &'a NSDate,
    zone: &'a NSTimeZone,
    calendar: &'a NSCalendar,
}

/// Section titles for the day an event is under way or next starts.
struct Days {
    calendar: Retained<NSCalendar>,
    today: usize,
    year: isize,
    same_year: Retained<NSDateFormatter>,
    other_year: Retained<NSDateFormatter>,
}

impl Days {
    /// Writes dates in `calendar`'s time zone.
    fn new(now: &NSDate, calendar: &NSCalendar) -> Self {
        let zone = calendar.timeZone();
        let formatter = |template: &str| {
            let formatter = NSDateFormatter::new();
            formatter.setTimeZone(Some(&zone));
            formatter.setLocalizedDateFormatFromTemplate(&NSString::from_str(template));
            formatter
        };
        Self {
            calendar: calendar.retain(),
            today: day_number(calendar, now),
            year: calendar.component_fromDate(NSCalendarUnit::Year, now),
            same_year: formatter("EEEEdMMMM"),
            other_year: formatter("EEEEdMMMMy"),
        }
    }

    fn at(&mut self, now: &NSDate) {
        self.today = day_number(&self.calendar, now);
        self.year = self.calendar.component_fromDate(NSCalendarUnit::Year, now);
    }

    fn title(&self, day: usize, date: &NSDate) -> String {
        match day.checked_sub(self.today) {
            Some(0) => "Today".into(),
            Some(1) => "Tomorrow".into(),
            _ => self.date(date),
        }
    }

    /// The weekday and date, with the year when it isn't this year.
    fn date(&self, date: &NSDate) -> String {
        if self.calendar.component_fromDate(NSCalendarUnit::Year, date) == self.year {
            self.same_year.stringFromDate(date).to_string()
        } else {
            self.other_year.stringFromDate(date).to_string()
        }
    }
}

fn day_number(calendar: &NSCalendar, date: &NSDate) -> usize {
    calendar.ordinalityOfUnit_inUnit_forDate(NSCalendarUnit::Day, NSCalendarUnit::Era, date)
}

/// The one section for events that have ended, which typed searches list last.
const ENDED: &str = "Earlier";

fn event_rows(mut events: Vec<Event>, terms: &[&str], limit: usize, window: &Window) -> Vec<Batch> {
    let days = Days::new(window.now, window.calendar);
    let zone = window.zone;
    let now_date = window.now;
    let now = window.now.timeIntervalSince1970();
    let start = window.start.timeIntervalSince1970();
    let end = window.end.timeIntervalSince1970();
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
    let times = Times::new(window.now, window.calendar, window.zone);
    let mut batches: Vec<Batch> = Vec::new();
    for event in events.into_iter().take(limit) {
        let occurrence = event.occurrence.as_ref().unwrap_or(&event.start);
        let id = format!("{}:{}", event.id, occurrence.timeIntervalSince1970());
        let url = event_url(&event, zone);
        let title = if event.title.trim().is_empty() {
            "Untitled event"
        } else {
            &event.title
        };
        let (section, day) = if chronological_key(&event).0 {
            (ENDED.to_owned(), None)
        } else {
            let date = if event.start.timeIntervalSince1970() < now {
                now_date
            } else {
                &event.start
            };
            let day = day_number(window.calendar, date);
            (days.title(day, date), Some(day))
        };
        let subtitle = format!("{} · {}", times.text(&event, day), event.calendar);
        let row = row(&id, title, subtitle, Some(("Open in Calendar", url)));
        match batches.last_mut() {
            Some(batch) if batch.section.as_deref() == Some(section.as_str()) => batch.items.push(row),
            _ => batches.push(Batch::section(section, vec![row])),
        }
    }
    batches
}

/// Formats when an event happens, leaving out what its section heading already says.
struct Times {
    calendar: Retained<NSCalendar>,
    zone: Retained<NSTimeZone>,
    year: isize,
    /// The locale's, unless set.
    locale: Option<Retained<NSLocale>>,
    /// One formatter for each template, since setting a template works the pattern out again.
    formatters: std::cell::RefCell<Vec<(&'static str, Retained<NSDateIntervalFormatter>)>>,
}

impl Times {
    fn new(now: &NSDate, calendar: &NSCalendar, zone: &NSTimeZone) -> Self {
        Self {
            calendar: calendar.retain(),
            zone: zone.retain(),
            year: calendar.component_fromDate(NSCalendarUnit::Year, now),
            locale: None,
            formatters: std::cell::RefCell::default(),
        }
    }

    fn between(&self, template: &'static str, from: &NSDate, to: &NSDate) -> String {
        let mut formatters = self.formatters.borrow_mut();
        let formatter = match formatters.iter().find(|(made, _)| *made == template) {
            Some((_, formatter)) => formatter,
            None => {
                let formatter = NSDateIntervalFormatter::new();
                formatter.setTimeZone(Some(&self.zone));
                formatter.setCalendar(Some(&self.calendar));
                formatter.setLocale(self.locale.as_deref());
                formatter.setDateTemplate(Some(&NSString::from_str(template)));
                formatters.push((template, formatter));
                &formatters.last().expect("just pushed").1
            }
        };
        formatter.stringFromDate_toDate(from, to).to_string()
    }

    fn at(&mut self, now: &NSDate) {
        self.year = self.calendar.component_fromDate(NSCalendarUnit::Year, now);
    }

    /// Whether the event starts and ends on one day, counting one that ends at midnight as
    /// the day before's.
    fn within_one_day(&self, event: &Event) -> bool {
        day_number(&self.calendar, &event.start) == day_number(&self.calendar, &last_moment(event))
    }

    /// Only the time when the event falls within `heading_day`, the day its section names;
    /// otherwise the dates too, with the year when it is not the current one.
    fn text(&self, event: &Event, heading_day: Option<usize>) -> String {
        let start = event.start.timeIntervalSince1970();
        let end = NSDate::dateWithTimeIntervalSince1970(event.end.timeIntervalSince1970().max(start));
        let last = last_moment(event);
        let first_day = day_number(&self.calendar, &event.start);
        let one_day = self.within_one_day(event);
        let between = |template, from: &NSDate, to: &NSDate| self.between(template, from, to);
        let range = |template, to: &NSDate| between(template, &event.start, to);
        if one_day && heading_day == Some(first_day) {
            return if event.all_day {
                "All day".into()
            } else if day_number(&self.calendar, &end) == first_day {
                range("jmm", &end)
            } else {
                // The formatter spells out both dates whenever the range crosses midnight.
                let time = |date: &NSDate| between("jmm", date, date);
                format!("{}\u{2009}–\u{2009}{}", time(&event.start), time(&end))
            };
        }
        let other_year = [&*event.start, &*last]
            .into_iter()
            .any(|date| self.calendar.component_fromDate(NSCalendarUnit::Year, date) != self.year);
        if event.all_day {
            format!(
                "{} · All day",
                range(if other_year { "dMMMy" } else { "dMMM" }, &last)
            )
        } else {
            range(if other_year { "dMMMyjmm" } else { "dMMMjmm" }, &end)
        }
    }
}

/// All-day events end at the start of the next day, and a timed event that ends at midnight
/// belongs to the day before.
fn last_moment(event: &Event) -> Retained<NSDate> {
    let start = event.start.timeIntervalSince1970();
    let end = event.end.timeIntervalSince1970().max(start);
    NSDate::dateWithTimeIntervalSince1970((end - 1.0).max(start))
}

/// Calendar links, which name an occurrence by a date stamp: a local one for floating all-day
/// occurrences, otherwise UTC.
struct Links {
    local: Retained<NSDateFormatter>,
    utc: Retained<NSDateFormatter>,
}

impl Links {
    fn new(zone: &NSTimeZone) -> Self {
        let stamp = |zone: &NSTimeZone| {
            let formatter = NSDateFormatter::new();
            formatter.setLocale(Some(&NSLocale::localeWithLocaleIdentifier(&NSString::from_str(
                "en_US_POSIX",
            ))));
            formatter.setTimeZone(Some(zone));
            formatter.setDateFormat(Some(&NSString::from_str("yyyyMMdd'T'HHmmss'Z'")));
            formatter
        };
        Self {
            local: stamp(zone),
            utc: stamp(&NSTimeZone::timeZoneForSecondsFromGMT(0)),
        }
    }

    fn url(&self, event: &Event) -> String {
        let id = crate::workspace::encode_url_component(&event.id);
        let occurrence = event.occurrence.as_ref().map_or_else(String::new, |date| {
            let formatter = if event.all_day { &self.local } else { &self.utc };
            format!("{}/", formatter.stringFromDate(date))
        });
        format!("ical://ekevent/{occurrence}{id}?method=show&options=more")
    }
}

fn event_url(event: &Event, zone: &NSTimeZone) -> String {
    Links::new(zone).url(event)
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
        ("Dentist", "High Street", "Personal", 180_000, 181_800, false),
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
        url: None,
        color: None,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(
        events: Vec<Event>,
        terms: &[&str],
        limit: usize,
        now: &NSDate,
        start: &NSDate,
        end: &NSDate,
        zone: &NSTimeZone,
    ) -> Vec<Batch> {
        let calendar = NSCalendar::currentCalendar();
        calendar.setTimeZone(zone);
        let window = Window {
            now,
            start,
            end,
            zone,
            calendar: &calendar,
        };
        event_rows(events, terms, limit, &window)
    }

    fn flat(batches: Vec<Batch>) -> Vec<Item> {
        batches.into_iter().flat_map(|batch| batch.items).collect()
    }

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
    fn upcoming_events_are_grouped_by_the_day_they_are_under_way() {
        let mut batches = Vec::new();
        CalendarProvider::fixtures()
            .search_batches(&Request::detached("", 30), &mut |batch| batches.push(batch))
            .unwrap();
        let sections: Vec<_> = batches
            .iter()
            .map(|batch| {
                let titles: Vec<_> = batch.items.iter().map(Item::display_title).collect();
                (batch.section.clone().unwrap(), titles)
            })
            .collect();
        assert_eq!(sections.len(), 3, "{sections:?}");
        assert_eq!(
            sections[0],
            ("Today".into(), vec!["Project planning", "Design review"])
        );
        assert_eq!(sections[1], ("Tomorrow".into(), vec!["Out of office"]));
        assert!(sections[2].0.contains("25"), "{sections:?}");
        assert_eq!(sections[2].1, ["Dentist"]);
        assert!(batches.iter().all(|batch| batch.details.is_empty()));
    }

    #[test]
    fn typed_searches_put_ongoing_events_under_today_and_ended_ones_last() {
        let now = NSDate::dateWithTimeIntervalSince1970(FIXTURE_NOW);
        let zone = NSTimeZone::timeZoneForSecondsFromGMT(0);
        let start = now.dateByAddingTimeInterval(-30.0 * 86_400.0);
        let end = now.dateByAddingTimeInterval(400.0 * 86_400.0);
        let mut events = fixture_events(&now);
        events[0].start = now.dateByAddingTimeInterval(-2.0 * 86_400.0);
        events[1].start = now.dateByAddingTimeInterval(-86_400.0);
        events[1].end = now.dateByAddingTimeInterval(-82_800.0);
        events[3].start = now.dateByAddingTimeInterval(365.0 * 86_400.0);
        events[3].end = events[3].start.dateByAddingTimeInterval(1800.0);
        let batches = rows(events, &[], 30, &now, &start, &end, &zone);
        let sections: Vec<_> = batches
            .iter()
            .map(|batch| (batch.section.clone().unwrap(), batch.items.len()))
            .collect();
        assert_eq!(sections[0], ("Today".into(), 1));
        assert_eq!(sections[1], ("Tomorrow".into(), 1));
        assert!(sections[2].0.contains("2027"), "{sections:?}");
        assert_eq!(sections[3], (ENDED.into(), 1));
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
        events[3].start = now.dateByAddingTimeInterval(-7200.0);
        events[3].end = start.dateByAddingTimeInterval(-1.0);
        let rows = flat(rows(events, &[], 30, &now, &start, &end, &zone));
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
                    url: None,
                    color: None,
                }
            })
            .collect();
        let rows = flat(rows(events, &["standup"], 4, &now, &start, &end, &zone));
        let expected =
            [-900.0, 3600.0, 7200.0, -3600.0].map(|offset| format!("standup:{}", FIXTURE_NOW + offset));
        assert_eq!(rows.into_iter().map(|row| row.id).collect::<Vec<_>>(), expected);
    }

    /// Runs `check` with fixture events and an en_GB `Times` whose zone is `offset` from GMT.
    fn with_times(offset: isize, check: impl FnOnce(&Times, &NSDate, Vec<Event>)) {
        let now = NSDate::dateWithTimeIntervalSince1970(FIXTURE_NOW);
        let zone = NSTimeZone::timeZoneForSecondsFromGMT(offset);
        let calendar = NSCalendar::currentCalendar();
        calendar.setTimeZone(&zone);
        let window = Window {
            now: &now,
            start: &now,
            end: &now,
            zone: &zone,
            calendar: &calendar,
        };
        let mut times = Times::new(window.now, window.calendar, window.zone);
        times.locale = Some(NSLocale::localeWithLocaleIdentifier(&NSString::from_str("en_GB")));
        check(&times, &now, fixture_events(&now));
    }

    fn day_of(times: &Times, event: &Event) -> Option<usize> {
        Some(day_number(&times.calendar, &event.start))
    }

    #[test]
    fn times_use_the_output_zone() {
        with_times(-8 * 3600, |times, _, events| {
            let timed = times.text(&events[0], None);
            assert!(timed.contains("02:00") && timed.contains("03:00"), "{timed}");
        });
    }

    #[test]
    fn rows_under_their_own_day_show_only_the_time() {
        with_times(0, |times, now, mut events| {
            let timed = times.text(&events[0], day_of(times, &events[0]));
            assert_eq!(timed, "10:00\u{2009}–\u{2009}11:00");
            assert_eq!(times.text(&events[2], day_of(times, &events[2])), "All day");

            events[1].end = now.dateByAddingTimeInterval(16.0 * 3600.0);
            let until_midnight = times.text(&events[1], day_of(times, &events[1]));
            assert_eq!(until_midnight, "16:00\u{2009}–\u{2009}00:00");
        });
    }

    #[test]
    fn rows_without_their_day_as_heading_show_the_dates() {
        with_times(0, |times, now, mut events| {
            let today = day_of(times, &events[0]);
            let earlier = times.text(&events[0], None);
            assert!(
                earlier.contains("23 Nov") && earlier.contains("10:00"),
                "{earlier}"
            );
            assert!(!earlier.contains("2026"), "{earlier}");

            let all_day = times.text(&events[2], None);
            assert!(all_day.contains("24 Nov") && !all_day.contains("25"), "{all_day}");
            assert!(all_day.ends_with(" · All day"), "{all_day}");

            let tomorrow = day_of(times, &events[2]);
            events[2].end = events[2].end.dateByAddingTimeInterval(86_400.0);
            let days = times.text(&events[2], tomorrow);
            assert!(days.contains("24") && days.contains("25 Nov"), "{days}");
            assert!(days.ends_with(" · All day"), "{days}");

            events[1].start = now.dateByAddingTimeInterval(14.0 * 3600.0);
            events[1].end = now.dateByAddingTimeInterval(18.0 * 3600.0);
            let overnight = times.text(&events[1], today);
            assert!(
                overnight.contains("23 Nov") && overnight.contains("24 Nov"),
                "{overnight}"
            );

            events[3].start = now.dateByAddingTimeInterval(365.0 * 86_400.0);
            events[3].end = events[3].start.dateByAddingTimeInterval(1800.0);
            let next_year = times.text(&events[3], None);
            assert!(next_year.contains("2027"), "{next_year}");
        });
    }

    #[test]
    fn upcoming_rows_leave_the_date_to_their_heading() {
        let mut batches = Vec::new();
        CalendarProvider::fixtures()
            .search_batches(&Request::detached("", 30), &mut |batch| batches.push(batch))
            .unwrap();
        for item in batches.iter().flat_map(|batch| &batch.items) {
            let View::Text { subtitle, .. } = &item.view else {
                panic!("expected a text row")
            };
            assert!(
                !subtitle.contains("Nov") && !subtitle.contains("2026"),
                "{subtitle}"
            );
        }
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

    #[test]
    fn the_extension_service_reads_fixture_events_within_the_declared_days() {
        use dupin_host_services::calendar::{Bounds, Calendar, Query};

        let service = Calendar::new(CalendarProvider::fixtures());
        let base = FIXTURE_NOW as Millis * 1000;
        let query = Query::new(base, base + 2 * 86_400_000, None, Some("review"), None).unwrap();
        assert_eq!(
            service.days(&query).unwrap(),
            Bounds {
                past_days: 0,
                future_days: 2
            }
        );
        let events = service
            .events(
                &query,
                Bounds {
                    past_days: 0,
                    future_days: 2,
                },
            )
            .unwrap();
        assert_eq!(
            events
                .iter()
                .map(|event| event.title.as_str())
                .collect::<Vec<_>>(),
            ["Design review"]
        );
        assert!(matches!(
            service.events(
                &query,
                Bounds {
                    past_days: 0,
                    future_days: 1,
                }
            ),
            Err(ServiceError::OutOfScope(_))
        ));
    }

    /// `event` as the extension service hands it over, written in en_GB and UTC.
    fn service_event(event: Event) -> service::Event {
        let now = NSDate::dateWithTimeIntervalSince1970(FIXTURE_NOW);
        let zone = NSTimeZone::timeZoneForSecondsFromGMT(0);
        let calendar = NSCalendar::currentCalendar();
        calendar.setTimeZone(&zone);
        let days = Days::new(&now, &calendar);
        let mut times = Times::new(&now, &calendar, &zone);
        times.locale = Some(NSLocale::localeWithLocaleIdentifier(&NSString::from_str("en_GB")));
        let mut events = [event.into_service()];
        describe(&mut events, &days, &times, &Links::new(&zone));
        let [event] = events;
        event
    }

    #[test]
    fn events_reach_extensions_in_milliseconds_with_occurrence_ids() {
        let now = NSDate::dateWithTimeIntervalSince1970(FIXTURE_NOW);
        let mut events = fixture_events(&now).into_iter();
        let mut planning = events.next().unwrap();
        planning.url = Some("https://example.com/plan".into());
        planning.color = Some("#1badf8".into());
        planning.occurrence = Some(now.clone());
        let base = FIXTURE_NOW as Millis * 1000;
        let planning = service_event(planning);
        assert_eq!(
            (
                planning.id.as_str(),
                planning.title.as_str(),
                planning.start,
                planning.end,
                planning.all_day,
                &planning.calendar,
                planning.location.as_deref(),
                planning.url.as_deref(),
            ),
            (
                format!("project-planning:{base}").as_str(),
                "Project planning",
                base + 7_200_000,
                base + 10_800_000,
                false,
                &CalendarInfo {
                    name: "Work".into(),
                    color: Some("#1badf8".into()),
                },
                Some("Studio"),
                Some("https://example.com/plan"),
            )
        );
        let holiday = service_event(events.nth(1).unwrap());
        assert!(holiday.all_day);
        assert_eq!(holiday.location, None);
        assert_eq!(holiday.start, base + 57_600_000);
        assert_eq!(millis(&date(holiday.start)), holiday.start);
    }

    #[test]
    fn events_carry_their_calendar_link_and_dates_as_the_mac_writes_them() {
        let now = NSDate::dateWithTimeIntervalSince1970(FIXTURE_NOW);
        let mut events = fixture_events(&now).into_iter();
        let mut planning = events.next().unwrap();
        planning.occurrence = Some(planning.start.clone());
        let planning = service_event(planning);
        assert_eq!(
            planning.link.as_deref(),
            Some("ical://ekevent/20261123T100000Z/project-planning?method=show&options=more")
        );
        assert_eq!(
            planning.labels.time.as_deref(),
            Some("10:00\u{2009}–\u{2009}11:00")
        );
        let dates = &planning.labels.dates;
        assert!(dates.contains("23 Nov") && dates.contains("10:00"), "{dates}");
        assert!(planning.labels.day.contains("23"), "{:?}", planning.labels);

        let holiday = service_event(events.nth(1).unwrap());
        assert_eq!(holiday.labels.time.as_deref(), Some("All day"));
        assert!(
            holiday.labels.dates.ends_with(" · All day"),
            "{:?}",
            holiday.labels
        );
        assert!(holiday.labels.day.contains("24"), "{:?}", holiday.labels);

        let mut overnight = events.next().unwrap();
        overnight.end = overnight.start.dateByAddingTimeInterval(86_400.0);
        let overnight = service_event(overnight);
        assert_eq!(overnight.labels.time, None);
        assert!(
            overnight.labels.dates.contains("26 Nov"),
            "{:?}",
            overnight.labels
        );
    }
}
