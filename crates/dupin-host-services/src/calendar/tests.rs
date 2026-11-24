use super::*;

const HOUR: Millis = 3_600_000;
const TODAY: Millis = 1_795_392_000_000;

fn event(id: &str, title: &str, calendar: &str, start: Millis, end: Millis) -> Event {
    Event {
        id: id.into(),
        title: title.into(),
        start,
        end,
        all_day: false,
        calendar: CalendarInfo {
            name: calendar.into(),
            color: Some("#1badf8".into()),
        },
        location: None,
        url: None,
        link: None,
        labels: Labels::default(),
    }
}

fn fixture(access: OsAccess) -> Fixture {
    Fixture {
        access,
        today: TODAY,
        events: vec![
            Event {
                location: Some("Meeting room".into()),
                url: Some("https://example.com/review".into()),
                ..event(
                    "review",
                    "Design review",
                    "Work",
                    TODAY + 10 * HOUR,
                    TODAY + 11 * HOUR,
                )
            },
            event(
                "planning",
                "Project planning",
                "Work",
                TODAY + 9 * HOUR,
                TODAY + 10 * HOUR,
            ),
            Event {
                all_day: true,
                ..event(
                    "holiday",
                    "Out of office",
                    "Personal",
                    TODAY + DAY,
                    TODAY + 2 * DAY,
                )
            },
            event(
                "dentist",
                "Dentist",
                "Personal",
                TODAY - 2 * DAY,
                TODAY - 2 * DAY + HOUR,
            ),
            event("far", "Conference", "Work", TODAY + 40 * DAY, TODAY + 41 * DAY),
        ],
    }
}

const WEEK: Bounds = Bounds {
    past_days: 7,
    future_days: 7,
};

fn titles(events: &[Event]) -> Vec<&str> {
    events.iter().map(|event| event.title.as_str()).collect()
}

#[test]
fn events_within_the_range_come_earliest_first() {
    let calendar = Calendar::new(fixture(OsAccess::Allowed));
    let query = Query::new(TODAY, TODAY + 2 * DAY, None, None, None).unwrap();
    let events = calendar.events(&query, WEEK).unwrap();
    assert_eq!(
        titles(&events),
        ["Project planning", "Design review", "Out of office"]
    );
    assert!(events[2].all_day);
}

#[test]
fn calendars_query_and_limit_narrow_the_events() {
    let calendar = Calendar::new(fixture(OsAccess::Allowed));
    let range = |calendars, query: Option<&str>, limit| {
        let query = Query::new(TODAY - 3 * DAY, TODAY + 3 * DAY, calendars, query, limit).unwrap();
        titles(&calendar.events(&query, WEEK).unwrap())
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        range(Some(vec!["Personal".into()]), None, None),
        ["Dentist", "Out of office"]
    );
    assert_eq!(range(None, Some(" ROOM design "), None), ["Design review"]);
    assert_eq!(range(None, Some("nothing"), None), Vec::<String>::new());
    assert_eq!(range(None, None, Some(2)), ["Dentist", "Project planning"]);
}

#[test]
fn a_latest_first_read_keeps_the_events_nearest_the_end_of_the_range() {
    let calendar = Calendar::new(fixture(OsAccess::Allowed));
    let query = Query::new(TODAY - 3 * DAY, TODAY + 3 * DAY, None, None, Some(2))
        .unwrap()
        .ordered(Order::Latest);
    assert_eq!(
        titles(&calendar.events(&query, WEEK).unwrap()),
        ["Out of office", "Design review"]
    );
}

#[test]
fn titles_come_as_one_line_that_fits_a_row() {
    let calendar = Calendar::new(Fixture {
        access: OsAccess::Allowed,
        today: TODAY,
        events: vec![
            event(
                "pasted",
                " Sync:\r\n\tweekly \u{0007} review ",
                "Work",
                TODAY,
                TODAY + HOUR,
            ),
            event(
                "long",
                &"会議".repeat(200),
                "Work",
                TODAY + HOUR,
                TODAY + 2 * HOUR,
            ),
        ],
    });
    let query = Query::new(TODAY, TODAY + DAY, None, Some("weekly"), None).unwrap();
    assert_eq!(
        titles(&calendar.events(&query, WEEK).unwrap()),
        ["Sync: weekly review"]
    );
    let query = Query::new(TODAY, TODAY + DAY, None, Some("会議"), None).unwrap();
    let [event] = calendar.events(&query, WEEK).unwrap().try_into().unwrap();
    assert!(event.title.len() <= MAX_TITLE_BYTES);
    assert!(event.title.starts_with("会議"));
}

#[test]
fn a_range_beyond_the_declared_window_is_out_of_scope() {
    let calendar = Calendar::new(fixture(OsAccess::Allowed));
    let far = Query::new(TODAY, TODAY + 41 * DAY, None, None, None).unwrap();
    assert_eq!(
        calendar.days(&far).unwrap(),
        Bounds {
            past_days: 0,
            future_days: 40
        }
    );
    assert!(matches!(
        calendar.events(&far, WEEK),
        Err(ServiceError::OutOfScope(_))
    ));

    let back = Query::new(TODAY - 8 * DAY, TODAY, None, None, None).unwrap();
    assert_eq!(
        calendar.days(&back).unwrap(),
        Bounds {
            past_days: 8,
            future_days: 0
        }
    );
    assert!(matches!(
        calendar.events(&back, WEEK),
        Err(ServiceError::OutOfScope(_))
    ));

    let today_only = Bounds {
        past_days: 0,
        future_days: 0,
    };
    let today = Query::new(TODAY, TODAY + DAY, None, None, None).unwrap();
    assert_eq!(calendar.days(&today).unwrap(), today_only);
    assert_eq!(calendar.events(&today, today_only).unwrap().len(), 2);
}

#[test]
fn bounds_come_from_the_permit_scope() {
    assert_eq!(
        Bounds::of(&Scope::Calendar {
            past_days: 1,
            future_days: 30
        }),
        Some(Bounds {
            past_days: 1,
            future_days: 30
        })
    );
    assert_eq!(Bounds::of(&Scope::Dictionary), None);
}

#[test]
fn reads_without_full_access_return_structured_errors_and_never_reach_events() {
    struct NoEvents(OsAccess);
    impl Calendars for NoEvents {
        fn access(&self) -> OsAccess {
            self.0
        }
        fn day_offset(&self, _: Millis) -> Result<i64, ServiceError> {
            panic!("checked the range before access")
        }
        fn events(&self, _: Millis, _: Millis) -> Result<Vec<Event>, ServiceError> {
            panic!("read events without access")
        }
    }
    let query = Query::new(TODAY, TODAY + DAY, None, None, None).unwrap();
    for (access, expected) in [
        (
            OsAccess::NotDetermined,
            ServiceError::NeedsPermission(OsPermission::Calendars),
        ),
        (
            OsAccess::Denied,
            ServiceError::PermissionDenied(OsPermission::Calendars),
        ),
        (
            OsAccess::Restricted,
            ServiceError::PermissionRestricted(OsPermission::Calendars),
        ),
    ] {
        assert_eq!(
            Calendar::new(NoEvents(access)).events(&query, WEEK),
            Err(expected)
        );
    }
    let needs = serde_json::to_value(ServiceError::NeedsPermission(OsPermission::Calendars)).unwrap();
    assert_eq!(needs["code"], "needs-permission");
    assert_eq!(needs["permission"], "calendars");
    assert!(needs["message"].as_str().unwrap().contains("Request Access"));
}

#[test]
fn invalid_queries_are_refused() {
    assert!(Query::new(TODAY, TODAY, None, None, None).is_err());
    assert!(Query::new(TODAY + 1, TODAY, None, None, None).is_err());
    assert!(Query::new(TODAY, TODAY + 1, Some(vec![]), None, None).is_err());
    assert!(Query::new(TODAY, TODAY + 1, Some(vec![String::new()]), None, None).is_err());
    assert!(
        Query::new(
            TODAY,
            TODAY + 1,
            Some(vec!["c".into(); MAX_CALENDARS + 1]),
            None,
            None
        )
        .is_err()
    );
    assert!(
        Query::new(
            TODAY,
            TODAY + 1,
            None,
            Some(&"q".repeat(MAX_QUERY_CHARS + 1)),
            None
        )
        .is_err()
    );
    assert!(Query::new(TODAY, TODAY + 1, None, None, Some(0)).is_err());
    assert!(Query::new(TODAY, TODAY + 1, None, None, Some(MAX_EVENTS + 1)).is_err());
    assert!(Query::new(TODAY, TODAY + 1, None, None, Some(MAX_EVENTS)).is_ok());
}

#[test]
fn events_serialize_for_javascript_within_their_bounds() {
    let calendar = Calendar::new(Fixture {
        access: OsAccess::Allowed,
        today: TODAY,
        events: vec![Event {
            location: Some("  ".into()),
            url: Some(format!("https://example.com/{}", "a".repeat(MAX_URL_BYTES))),
            link: Some(format!("ical://ekevent/{}", "a".repeat(MAX_URL_BYTES))),
            calendar: CalendarInfo {
                name: "Work".into(),
                color: Some("red".into()),
            },
            ..event("long", &"é".repeat(MAX_TEXT_BYTES), "Work", TODAY, TODAY + HOUR)
        }],
    });
    let query = Query::new(TODAY, TODAY + DAY, None, None, None).unwrap();
    let [event] = calendar.events(&query, WEEK).unwrap().try_into().unwrap();
    assert!(event.title.len() <= MAX_TITLE_BYTES);
    assert!(event.title.chars().all(|ch| ch == 'é'));
    assert_eq!(event.location, None);
    assert_eq!(event.url, None);
    assert_eq!(event.link, None);
    assert_eq!(event.calendar.color, None);

    let json = serde_json::to_value(&fixture(OsAccess::Allowed).events[0]).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "id": "review",
            "title": "Design review",
            "start": TODAY + 10 * HOUR,
            "end": TODAY + 11 * HOUR,
            "allDay": false,
            "calendar": { "name": "Work", "color": "#1badf8" },
            "location": "Meeting room",
            "url": "https://example.com/review",
            "link": null,
            "labels": { "day": "", "time": null, "dates": "" },
        })
    );
}

#[test]
fn colors_are_srgb_hex() {
    assert_eq!(hex_color(1.0, 0.5, 0.0), "#ff8000");
    assert_eq!(hex_color(-1.0, 2.0, 0.1), "#00ff1a");
}
