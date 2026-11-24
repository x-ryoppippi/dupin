//! The `calendar` capability: reading events within the window the package declares.
//!
//! Reading needs macOS Calendars access. The service only reads its status, which never
//! prompts: while macOS hasn't been asked, a read fails with
//! [`ServiceError::NeedsPermission`], and only the app asks, from a deliberate action such as
//! Request Access, following the shared Calendar access policy.

use dupin_broker::{OsAccess, OsPermission, Scope};
use serde::{Deserialize, Serialize};

use crate::service::{ServiceError, bounded};

pub const DEFAULT_EVENTS: usize = 100;
pub const MAX_EVENTS: usize = 500;
pub const MAX_QUERY_CHARS: usize = 200;
pub const MAX_CALENDARS: usize = 32;
pub const MAX_CALENDAR_NAME_BYTES: usize = 256;
/// Each location and calendar name.
pub const MAX_TEXT_BYTES: usize = 1024;
/// What an extension's row title holds, so an event's title fits its row as it comes.
pub const MAX_TITLE_BYTES: usize = 512;
pub const MAX_URL_BYTES: usize = 2048;

pub const PERMISSION: OsPermission = OsPermission::Calendars;

/// Milliseconds since the Unix epoch, as JavaScript's `Date` counts.
pub type Millis = i64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    /// Stable for one occurrence of a recurring event.
    pub id: String,
    /// One line, up to [`MAX_TITLE_BYTES`].
    pub title: String,
    pub start: Millis,
    pub end: Millis,
    pub all_day: bool,
    pub calendar: CalendarInfo,
    pub location: Option<String>,
    pub url: Option<String>,
    /// Opens this occurrence in the Mac's Calendar.
    pub link: Option<String>,
    pub labels: Labels,
}

/// The event's dates as this Mac's locale writes them, in its time zone, which a guest
/// without `Intl` can't do itself.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Labels {
    /// The day it starts, such as “Wednesday 25 November”, with the year when that isn't
    /// this year.
    pub day: String,
    /// Only its time, such as “10:00–11:00”, or “All day”, when it falls within one day.
    pub time: Option<String>,
    /// Its dates as well, with the year when either isn't this year.
    pub dates: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarInfo {
    pub name: String,
    /// `#rrggbb` in sRGB.
    pub color: Option<String>,
}

/// Which end of the range a read's limit keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Order {
    /// Earliest start first.
    #[default]
    Earliest,
    /// Latest start first, for the events nearest the end of a range in the past.
    Latest,
}

/// A validated read: events overlapping `start..end`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    start: Millis,
    end: Millis,
    calendars: Option<Vec<String>>,
    terms: Vec<String>,
    limit: usize,
    order: Order,
}

impl Query {
    pub fn new(
        start: Millis,
        end: Millis,
        calendars: Option<Vec<String>>,
        query: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Self, ServiceError> {
        if start >= end {
            return Err(ServiceError::Invalid("the range must end after it starts".into()));
        }
        if let Some(names) = &calendars
            && (names.is_empty()
                || names.len() > MAX_CALENDARS
                || names
                    .iter()
                    .any(|name| name.is_empty() || name.len() > MAX_CALENDAR_NAME_BYTES))
        {
            return Err(ServiceError::Invalid(format!(
                "name 1–{MAX_CALENDARS} calendars of 1–{MAX_CALENDAR_NAME_BYTES} bytes, or leave them out"
            )));
        }
        let query = query.unwrap_or_default();
        if query.chars().count() > MAX_QUERY_CHARS {
            return Err(ServiceError::Invalid(format!(
                "a query is at most {MAX_QUERY_CHARS} characters"
            )));
        }
        let limit = limit.unwrap_or(DEFAULT_EVENTS);
        if !(1..=MAX_EVENTS).contains(&limit) {
            return Err(ServiceError::Invalid(format!("the limit is 1–{MAX_EVENTS}")));
        }
        Ok(Self {
            start,
            end,
            calendars,
            terms: query
                .to_lowercase()
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
            limit,
            order: Order::Earliest,
        })
    }

    pub fn ordered(self, order: Order) -> Self {
        Self { order, ..self }
    }
}

/// The window a package declares, in whole days around today.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    pub past_days: u32,
    pub future_days: u32,
}

impl Bounds {
    /// The bounds of a permit's calendar scope.
    pub fn of(scope: &Scope) -> Option<Self> {
        match *scope {
            Scope::Calendar {
                past_days,
                future_days,
            } => Some(Self {
                past_days,
                future_days,
            }),
            _ => None,
        }
    }
}

/// Where events come from: EventKit on macOS, [`Fixture`] in tests.
pub trait Calendars: Send + Sync {
    /// macOS's answer for Calendars access, read without prompting. Write-only access counts
    /// as denied.
    fn access(&self) -> OsAccess;
    /// Whole days from today to the day `at` falls on, in the user's calendar and time zone.
    fn day_offset(&self, at: Millis) -> Result<i64, ServiceError>;
    /// Events overlapping `start..end`, in any order.
    fn events(&self, start: Millis, end: Millis) -> Result<Vec<Event>, ServiceError>;
    /// Fills in the `link` and `labels` of the events a read returns, once the rest are left
    /// out, since writing dates takes far longer than matching them.
    fn describe(&self, events: &mut [Event]) {
        let _ = events;
    }
}

pub struct Calendar<C> {
    backend: C,
}

impl<C: Calendars> Calendar<C> {
    pub fn new(backend: C) -> Self {
        Self { backend }
    }

    /// The days before and after today the query reaches, for the broker's
    /// `Request::CalendarRead`.
    pub fn days(&self, query: &Query) -> Result<Bounds, ServiceError> {
        let first = self.backend.day_offset(query.start)?;
        let last = self.backend.day_offset(query.end - 1)?;
        let days = |offset: i64| {
            u32::try_from(offset.max(0))
                .map_err(|_| ServiceError::OutOfScope("the range is too far from today".into()))
        };
        Ok(Bounds {
            past_days: days(-first)?,
            future_days: days(last)?,
        })
    }

    /// The events the query matches, in its order, within the declared `bounds`.
    pub fn events(&self, query: &Query, bounds: Bounds) -> Result<Vec<Event>, ServiceError> {
        if let Some(error) = ServiceError::from_access(PERMISSION, self.backend.access()) {
            return Err(error);
        }
        let reach = self.days(query)?;
        if reach.past_days > bounds.past_days || reach.future_days > bounds.future_days {
            return Err(ServiceError::OutOfScope(format!(
                "calendar reads are limited to {} days back and {} days ahead",
                bounds.past_days, bounds.future_days
            )));
        }
        let mut events: Vec<Event> = self
            .backend
            .events(query.start, query.end)?
            .into_iter()
            .filter(|event| {
                event.start < query.end && event.end.max(event.start.saturating_add(1)) > query.start
            })
            .filter(|event| {
                query
                    .calendars
                    .as_ref()
                    .is_none_or(|names| names.contains(&event.calendar.name))
            })
            .filter(|event| {
                let title = event.title.to_lowercase();
                let location = event.location.as_deref().unwrap_or_default().to_lowercase();
                query
                    .terms
                    .iter()
                    .all(|term| title.contains(term) || location.contains(term))
            })
            .collect();
        events.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.id.cmp(&b.id)));
        if query.order == Order::Latest {
            events.reverse();
        }
        events.truncate(query.limit);
        self.backend.describe(&mut events);
        Ok(events.into_iter().map(Event::bounded).collect())
    }
}

impl Event {
    fn bounded(self) -> Self {
        Self {
            id: bounded(self.id, MAX_TEXT_BYTES),
            title: bounded(one_line(&self.title), MAX_TITLE_BYTES),
            calendar: CalendarInfo {
                name: bounded(self.calendar.name, MAX_TEXT_BYTES),
                color: self.calendar.color.filter(|color| is_hex_color(color)),
            },
            location: self
                .location
                .filter(|location| !location.trim().is_empty())
                .map(|location| bounded(location, MAX_TEXT_BYTES)),
            url: self.url.filter(|url| url.len() <= MAX_URL_BYTES),
            link: self.link.filter(|link| link.len() <= MAX_URL_BYTES),
            ..self
        }
    }
}

/// `text` with each run of spaces, line breaks and other control characters as one space,
/// since a title pasted from an invitation may carry them.
fn one_line(text: &str) -> String {
    text.split(|ch: char| ch.is_whitespace() || ch.is_control())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_hex_color(color: &str) -> bool {
    color.len() == 7 && color.starts_with('#') && color[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// `#rrggbb` for sRGB components in 0–1.
pub fn hex_color(red: f64, green: f64, blue: f64) -> String {
    let byte = |component: f64| (component.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(red), byte(green), byte(blue))
}

const DAY: Millis = 86_400_000;

/// Fixed events in UTC, with `today` the start of the current day.
#[derive(Clone, Debug)]
pub struct Fixture {
    pub access: OsAccess,
    pub today: Millis,
    pub events: Vec<Event>,
}

impl Calendars for Fixture {
    fn access(&self) -> OsAccess {
        self.access
    }

    fn day_offset(&self, at: Millis) -> Result<i64, ServiceError> {
        Ok((at - self.today).div_euclid(DAY))
    }

    fn events(&self, start: Millis, end: Millis) -> Result<Vec<Event>, ServiceError> {
        Ok(self
            .events
            .iter()
            .filter(|event| event.start < end && event.end > start)
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests;
