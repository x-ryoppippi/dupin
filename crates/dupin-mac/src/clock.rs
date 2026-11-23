//! The local wall-clock time, in the user's time zone and calendar settings.

use dupin_core::snippets::LocalTime;
use objc2_foundation::{NSCalendar, NSCalendarUnit, NSDate};

pub fn now() -> LocalTime {
    let parts = NSCalendar::autoupdatingCurrentCalendar().components_fromDate(
        NSCalendarUnit::Year
            | NSCalendarUnit::Month
            | NSCalendarUnit::Day
            | NSCalendarUnit::Hour
            | NSCalendarUnit::Minute,
        &NSDate::now(),
    );
    let small = |value: isize| u8::try_from(value).unwrap_or_default();
    LocalTime {
        year: i32::try_from(parts.year()).unwrap_or_default(),
        month: small(parts.month()),
        day: small(parts.day()),
        hour: small(parts.hour()),
        minute: small(parts.minute()),
    }
}
