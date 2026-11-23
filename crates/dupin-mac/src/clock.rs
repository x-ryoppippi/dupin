//! Gregorian wall-clock time in the user's time zone for ISO date placeholders.

use dupin_core::snippets::LocalTime;
use objc2_foundation::{NSCalendar, NSCalendarIdentifierGregorian, NSCalendarUnit, NSDate, NSTimeZone};

pub fn now() -> LocalTime {
    local_time(&NSDate::now(), &NSTimeZone::localTimeZone())
}

fn local_time(date: &NSDate, zone: &NSTimeZone) -> LocalTime {
    let calendar = NSCalendar::calendarWithIdentifier(unsafe { NSCalendarIdentifierGregorian })
        .expect("Foundation supports the Gregorian calendar");
    calendar.setTimeZone(zone);
    let parts = calendar.components_fromDate(
        NSCalendarUnit::Year
            | NSCalendarUnit::Month
            | NSCalendarUnit::Day
            | NSCalendarUnit::Hour
            | NSCalendarUnit::Minute,
        date,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_placeholders_use_the_gregorian_date_in_the_local_zone() {
        let date = NSDate::dateWithTimeIntervalSince1970(0.0);
        let zone = NSTimeZone::timeZoneForSecondsFromGMT(-8 * 3600);
        let time = local_time(&date, &zone);
        assert_eq!(
            dupin_core::snippets::expand("{date} {datetime}", time, ""),
            "1969-12-31 1969-12-31 16:00",
        );
    }
}
