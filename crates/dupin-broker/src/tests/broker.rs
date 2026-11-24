use std::path::PathBuf;

use super::*;

const R1: &str = "sha256:1111";
const R2: &str = "sha256:2222";

fn installed(capabilities: Capabilities) -> (Broker, PackageId) {
    let mut broker = Broker::new(Base::default(), Local::default());
    let package = id("bookmarks");
    let _ = broker.install(package.clone(), rev(R1), capabilities);
    (broker, package)
}

fn enabled(capabilities: Capabilities) -> (Broker, PackageId) {
    let (mut broker, package) = installed(capabilities);
    broker.set_enabled(&package, true).unwrap();
    (broker, package)
}

fn get(url: &str) -> Request {
    Request::Http {
        url: url.into(),
        method: Method::Get,
    }
}

fn post(url: &str) -> Request {
    Request::Http {
        url: url.into(),
        method: Method::Post,
    }
}

const API: &str = "https://api.github.com/repos/x-ryoppippi/dupin";

fn check(broker: &Broker, package: &PackageId, request: &Request, trigger: Trigger) -> Decision {
    broker.check(package, &rev(R1), request, trigger)
}

#[track_caller]
fn permit(decision: Decision) -> Permit {
    match decision {
        Decision::Allow(permit) => permit,
        other => panic!("expected a permit, got {other:?}"),
    }
}

#[track_caller]
fn denial(decision: Decision) -> Denial {
    match decision {
        Decision::Deny(denial) => denial,
        other => panic!("expected a denial, got {other:?}"),
    }
}

#[track_caller]
fn consent_needed(decision: Decision) -> ConsentNeeded {
    match decision {
        Decision::NeedsConsent(needed) => needed,
        other => panic!("expected a consent request, got {other:?}"),
    }
}

fn github() -> Capabilities {
    caps(json!({ "http": [{ "origin": "https://api.github.com", "methods": ["GET", "POST"] }] }))
}

#[test]
fn a_new_installation_starts_off_and_ungranted() {
    let (mut broker, package) = installed(github());
    assert!(!broker.is_enabled(&package));
    assert_eq!(
        denial(check(&broker, &package, &get(API), Trigger::Activation)),
        Denial::Disabled
    );
    broker.set_enabled(&package, true).unwrap();
    assert_eq!(
        consent_needed(check(&broker, &package, &get(API), Trigger::Query)),
        ConsentNeeded {
            revision: rev(R1),
            scope: http("https://api.github.com", &[Method::Get, Method::Post]),
            reason: ConsentReason::NotGranted,
            may_prompt: false,
        }
    );
    assert!(consent_needed(check(&broker, &package, &get(API), Trigger::Activation)).may_prompt);
    assert!(!consent_needed(check(&broker, &package, &get(API), Trigger::Background)).may_prompt);
    assert_eq!(
        broker.consents(&package),
        [(
            http("https://api.github.com", &[Method::Get, Method::Post]),
            ConsentState::Undecided
        )]
    );
}

#[test]
fn a_granted_scope_allows_only_what_it_declares() {
    let (mut broker, package) = enabled(github());
    broker
        .approve(&package, &key("http:https://api.github.com"), None)
        .unwrap();
    let granted = permit(check(&broker, &package, &get(API), Trigger::Query));
    assert!(broker.validate(&granted));
    assert_eq!(granted.package(), &package);
    assert!(matches!(granted.scope(), Scope::Http { .. }));
    let upper = get("HTTPS://API.GITHUB.COM:443/x");
    assert!(matches!(
        check(&broker, &package, &upper, Trigger::Query),
        Decision::Allow(_)
    ));
    assert_eq!(
        denial(check(
            &broker,
            &package,
            &get("https://github.com/"),
            Trigger::Query
        )),
        Denial::Undeclared(key("http:https://github.com"))
    );
    assert_eq!(
        denial(check(
            &broker,
            &package,
            &get("https://api.github.com:8443/"),
            Trigger::Query
        )),
        Denial::Undeclared(key("http:https://api.github.com:8443"))
    );
    assert!(matches!(
        denial(check(
            &broker,
            &package,
            &get("http://api.github.com/"),
            Trigger::Query
        )),
        Denial::Invalid(_)
    ));
    let delete = Request::Http {
        url: API.into(),
        method: Method::Delete,
    };
    assert!(matches!(
        denial(check(&broker, &package, &delete, Trigger::Activation)),
        Denial::OutOfScope { .. }
    ));
}

#[test]
fn mutations_and_programs_need_a_user_activation() {
    let (mut broker, package) = enabled(everything());
    for scope in [
        "http:https://api.github.com",
        "clipboard:read",
        "clipboard:write",
        "clipboard:watch",
        "script:open-in-zed",
    ] {
        broker.approve(&package, &key(scope), None).unwrap();
    }
    broker
        .approve(
            &package,
            &key("exec:git-status"),
            Some(PathBuf::from("/usr/bin/git")),
        )
        .unwrap();
    broker.set_os_access(OsPermission::Pasteboard, OsAccess::Allowed);
    for request in [
        post(API),
        Request::Exec {
            profile: name("git-status"),
        },
        Request::Script {
            name: name("open-in-zed"),
            output: false,
        },
        Request::Clipboard(ClipboardAccess::Write),
        Request::Clipboard(ClipboardAccess::Read),
    ] {
        for trigger in [Trigger::Query, Trigger::Background] {
            assert_eq!(
                denial(check(&broker, &package, &request, trigger)),
                Denial::NeedsActivation,
                "{request:?}"
            );
        }
        assert!(
            matches!(
                check(&broker, &package, &request, Trigger::Activation),
                Decision::Allow(_)
            ),
            "{request:?}"
        );
    }
    let watch = Request::Clipboard(ClipboardAccess::Watch);
    assert!(matches!(
        check(&broker, &package, &watch, Trigger::Background),
        Decision::Allow(_)
    ));
    assert!(matches!(
        check(&broker, &package, &get(API), Trigger::Background),
        Decision::Allow(_)
    ));
    let output = Request::Script {
        name: name("open-in-zed"),
        output: true,
    };
    assert!(matches!(
        denial(check(&broker, &package, &output, Trigger::Activation)),
        Denial::OutOfScope { .. }
    ));
}

#[test]
fn files_scopes_are_bound_to_the_picked_root_and_stay_inside_it() {
    let (mut broker, package) = enabled(caps(json!({
        "files": { "notes": { "kind": "directory", "access": ["read"] } }
    })));
    let scope = key("files:notes");
    assert_eq!(
        broker.approve(&package, &scope, None),
        Err(BrokerError::BindingRequired(scope.clone()))
    );
    assert_eq!(
        broker.approve(&package, &scope, Some("Notes".into())),
        Err(BrokerError::RelativeBinding(scope.clone()))
    );
    broker
        .approve(&package, &scope, Some("/Users/me/Notes".into()))
        .unwrap();
    let read = |path: &str, access| Request::Files {
        scope: name("notes"),
        access,
        path: path.into(),
    };
    let granted = permit(check(
        &broker,
        &package,
        &read("a/b.md", FilesAccess::Read),
        Trigger::Query,
    ));
    assert_eq!(granted.binding(), Some(std::path::Path::new("/Users/me/Notes")));
    assert!(matches!(
        check(&broker, &package, &read("", FilesAccess::Read), Trigger::Query),
        Decision::Allow(_)
    ));
    for escape in ["../secret", "/etc/passwd", "a/../../b"] {
        assert!(
            matches!(
                denial(check(
                    &broker,
                    &package,
                    &read(escape, FilesAccess::Read),
                    Trigger::Query
                )),
                Denial::Invalid(_)
            ),
            "{escape}"
        );
    }
    assert!(matches!(
        denial(check(
            &broker,
            &package,
            &read("", FilesAccess::List),
            Trigger::Query
        )),
        Denial::OutOfScope { .. }
    ));
    assert_eq!(
        broker.approve(&package, &scope, Some("/Users/me/Other".into())),
        Ok(())
    );
    assert!(
        !broker.validate(&granted),
        "rebinding drops work bound to the old root"
    );
    let other = Scope::Files {
        name: name("other"),
        kind: FilesKind::Directory,
        access: [FilesAccess::Read].into(),
    };
    assert_eq!(
        broker.consent(&package, &rev(R1), &other, Some("/x".into())),
        Err(BrokerError::Undeclared(key("files:other")))
    );
    assert_eq!(
        broker.consent(
            &package,
            &rev(R1),
            &http("https://a.example", &[Method::Get]),
            None
        ),
        Err(BrokerError::Undeclared(key("http:https://a.example")))
    );
}

#[test]
fn storage_writes_stay_within_the_declared_quota() {
    let (mut broker, package) = enabled(caps(json!({ "storage": { "quotaBytes": 100 } })));
    assert_eq!(
        broker.approve(&package, &ScopeKey::Storage, Some("/x".into())),
        Err(BrokerError::UnexpectedBinding(ScopeKey::Storage))
    );
    broker.approve(&package, &ScopeKey::Storage, None).unwrap();
    let storage = |op, size_after| Request::Storage { op, size_after };
    assert!(matches!(
        check(&broker, &package, &storage(StorageOp::Write, 100), Trigger::Query),
        Decision::Allow(_)
    ));
    assert!(matches!(
        denial(check(
            &broker,
            &package,
            &storage(StorageOp::Write, 101),
            Trigger::Query
        )),
        Denial::OutOfScope { .. }
    ));
    assert!(matches!(
        check(
            &broker,
            &package,
            &storage(StorageOp::Read, 500),
            Trigger::Background
        ),
        Decision::Allow(_)
    ));
    assert!(matches!(
        check(&broker, &package, &storage(StorageOp::Delete, 0), Trigger::Query),
        Decision::Allow(_)
    ));
}

#[test]
fn calendar_reads_stay_within_their_window_and_need_os_access() {
    let (mut broker, package) = enabled(caps(json!({
        "calendar": { "access": ["read"], "pastDays": 1, "futureDays": 30 }
    })));
    let read = |past_days, future_days| Request::CalendarRead {
        past_days,
        future_days,
    };
    assert!(matches!(
        denial(check(&broker, &package, &read(2, 30), Trigger::Query)),
        Denial::OutOfScope { .. }
    ));
    assert!(matches!(
        denial(check(&broker, &package, &read(0, 31), Trigger::Query)),
        Denial::OutOfScope { .. }
    ));
    assert_eq!(
        consent_needed(check(&broker, &package, &read(0, 7), Trigger::Query)).reason,
        ConsentReason::NotGranted
    );
    broker.set_os_access(OsPermission::Calendars, OsAccess::Allowed);
    assert_eq!(
        consent_needed(check(&broker, &package, &read(0, 7), Trigger::Query)).reason,
        ConsentReason::NotGranted,
        "macOS access alone grants no extension authority"
    );
    broker.set_os_access(OsPermission::Calendars, OsAccess::NotDetermined);
    broker.approve(&package, &ScopeKey::Calendar, None).unwrap();
    let needed = consent_needed(check(&broker, &package, &read(1, 30), Trigger::Activation));
    assert_eq!(needed.reason, ConsentReason::OsAccess(OsPermission::Calendars));
    assert!(needed.may_prompt);
    assert!(!consent_needed(check(&broker, &package, &read(1, 30), Trigger::Query)).may_prompt);
    broker.set_os_access(OsPermission::Calendars, OsAccess::Allowed);
    let granted = permit(check(&broker, &package, &read(1, 30), Trigger::Query));
    assert!(broker.validate(&granted));
    let before = broker.generation(&package).unwrap();
    broker.set_os_access(OsPermission::Calendars, OsAccess::Denied);
    assert!(!broker.validate(&granted));
    assert!(broker.generation(&package).unwrap() > before);
    assert_eq!(
        denial(check(&broker, &package, &read(0, 1), Trigger::Activation)),
        Denial::OsDenied(OsPermission::Calendars)
    );
    broker.set_os_access(OsPermission::Calendars, OsAccess::Restricted);
    assert_eq!(
        denial(check(&broker, &package, &read(0, 1), Trigger::Activation)),
        Denial::OsRestricted(OsPermission::Calendars)
    );
    broker.set_os_access(OsPermission::Calendars, OsAccess::Allowed);
    assert!(
        !broker.validate(&granted),
        "access granted again doesn't revive old permits"
    );
}

#[test]
fn shared_os_access_never_grants_another_package() {
    let calendar = caps(json!({ "calendar": { "access": ["read"], "pastDays": 0, "futureDays": 1 } }));
    let (mut broker, first) = enabled(calendar.clone());
    let second = id("agenda");
    let _ = broker.install(second.clone(), rev(R1), calendar);
    broker.set_enabled(&second, true).unwrap();
    broker.approve(&first, &ScopeKey::Calendar, None).unwrap();
    broker.set_os_access(OsPermission::Calendars, OsAccess::Allowed);
    let read = Request::CalendarRead {
        past_days: 0,
        future_days: 1,
    };
    assert!(matches!(
        check(&broker, &first, &read, Trigger::Query),
        Decision::Allow(_)
    ));
    assert_eq!(
        consent_needed(check(&broker, &second, &read, Trigger::Query)).reason,
        ConsentReason::NotGranted
    );
}

#[test]
fn revoking_moves_the_package_to_a_new_generation_that_regranting_cannot_undo() {
    let (mut broker, package) = enabled(everything());
    let other = id("other");
    let _ = broker.install(other.clone(), rev(R1), github());
    let other_generation = broker.generation(&other).unwrap();
    let scope = key("http:https://api.github.com");
    broker.approve(&package, &scope, None).unwrap();
    broker.approve(&package, &ScopeKey::Dictionary, None).unwrap();
    let in_flight = permit(check(&broker, &package, &get(API), Trigger::Query));
    let dictionary = permit(check(
        &broker,
        &package,
        &Request::DictionaryLookup,
        Trigger::Query,
    ));
    let before = broker.generation(&package).unwrap();
    assert_eq!(in_flight.generation(), before);

    broker.revoke(&package, &scope).unwrap();
    let after = broker.generation(&package).unwrap();
    assert!(after > before);
    assert!(!broker.validate(&in_flight));
    assert!(
        !broker.validate(&dictionary),
        "cached results may mix scopes, so all are dropped"
    );
    assert_eq!(
        denial(check(&broker, &package, &get(API), Trigger::Activation)),
        Denial::Revoked(scope.clone())
    );
    assert_eq!(broker.generation(&other), Some(other_generation));

    broker.approve(&package, &scope, None).unwrap();
    assert!(!broker.validate(&in_flight));
    let fresh = permit(check(&broker, &package, &get(API), Trigger::Query));
    assert!(broker.validate(&fresh));
    assert!(fresh.generation() >= after);
}

#[test]
fn disabling_drops_work_and_reenabling_does_not_revive_it() {
    let (mut broker, package) = enabled(github());
    broker
        .approve(&package, &key("http:https://api.github.com"), None)
        .unwrap();
    let in_flight = permit(check(&broker, &package, &get(API), Trigger::Query));
    broker.set_enabled(&package, false).unwrap();
    assert!(!broker.validate(&in_flight));
    broker.set_enabled(&package, true).unwrap();
    assert!(!broker.validate(&in_flight));
    assert!(matches!(
        check(&broker, &package, &get(API), Trigger::Query),
        Decision::Allow(_)
    ));
}

#[test]
fn a_new_revision_keeps_covered_grants_asks_again_for_wider_ones_and_drops_removed_ones() {
    let (mut broker, package) = enabled(caps(json!({
        "http": [
            { "origin": "https://api.github.com", "methods": ["GET"] },
            { "origin": "https://example.com", "methods": ["GET", "POST"] }
        ],
        "clipboard": ["write"],
        "dictionary": ["lookup"]
    })));
    for scope in [
        "http:https://api.github.com",
        "http:https://example.com",
        "dictionary",
    ] {
        broker.approve(&package, &key(scope), None).unwrap();
    }
    broker.revoke(&package, &key("clipboard:write")).unwrap();
    let old = permit(check(&broker, &package, &get(API), Trigger::Query));

    let report = broker.install(
        package.clone(),
        rev(R2),
        caps(json!({
            "http": [
                { "origin": "https://api.github.com", "methods": ["GET", "POST"] },
                { "origin": "https://example.com", "methods": ["GET"] }
            ],
            "calendar": { "access": ["read"], "pastDays": 0, "futureDays": 1 }
        })),
    );
    assert_eq!(report.kept, [key("http:https://example.com")]);
    assert_eq!(report.changed, [key("http:https://api.github.com")]);
    assert_eq!(report.dropped, [key("dictionary")]);

    assert!(!broker.validate(&old));
    assert_eq!(
        denial(check(&broker, &package, &get(API), Trigger::Query)),
        Denial::StaleRevision
    );
    let new = |request: &Request| broker.check(&package, &rev(R2), request, Trigger::Activation);
    assert_eq!(consent_needed(new(&get(API))).reason, ConsentReason::Changed);
    assert!(matches!(new(&get("https://example.com/")), Decision::Allow(_)));
    assert!(matches!(
        denial(new(&post("https://example.com/"))),
        Denial::OutOfScope { .. }
    ));
    assert_eq!(
        consent_needed(new(&Request::CalendarRead {
            past_days: 0,
            future_days: 1
        }))
        .reason,
        ConsentReason::NotGranted
    );
    assert_eq!(
        denial(new(&Request::DictionaryLookup)),
        Denial::Undeclared(ScopeKey::Dictionary)
    );
    assert_eq!(
        broker
            .consents(&package)
            .into_iter()
            .map(|(scope, state)| (scope.key(), state))
            .collect::<Vec<_>>(),
        [
            (key("http:https://api.github.com"), ConsentState::Changed),
            (
                key("http:https://example.com"),
                ConsentState::Granted(GrantSource::Local)
            ),
            (ScopeKey::Calendar, ConsentState::Undecided),
        ]
    );

    let report = broker.install(
        package.clone(),
        rev(R2),
        caps(json!({
            "http": [{ "origin": "https://example.com", "methods": ["GET", "POST"] }],
            "clipboard": ["write"]
        })),
    );
    assert_eq!(
        report.changed,
        [key("http:https://example.com")],
        "a narrowed approval doesn't stretch back"
    );
    let new = |request: &Request| broker.check(&package, &rev(R2), request, Trigger::Activation);
    assert_eq!(
        denial(new(&Request::Clipboard(ClipboardAccess::Write))),
        Denial::Revoked(key("clipboard:write")),
        "a revocation outlives the declaration it revoked"
    );
}

#[test]
fn a_bundled_package_that_drops_and_redeclares_a_scope_stays_revoked() {
    let mut broker = Broker::new(Base::default(), Local::default());
    let package = id("calendar-tools");
    let clipboard = || caps(json!({ "clipboard": ["write"] }));
    broker.set_bundled(package.clone(), rev(R1));
    let _ = broker.install(package.clone(), rev(R1), clipboard());
    broker.revoke(&package, &key("clipboard:write")).unwrap();

    let edited = broker.install(
        package.clone(),
        rev(R2),
        caps(json!({ "dictionary": ["lookup"] })),
    );
    assert!(edited.dropped.is_empty());
    let _ = broker.install(package.clone(), rev(R1), clipboard());
    assert_eq!(
        broker.consents(&package),
        [(
            Scope::Clipboard {
                access: ClipboardAccess::Write
            },
            ConsentState::Revoked
        )]
    );
    assert_eq!(
        denial(check(
            &broker,
            &package,
            &Request::Clipboard(ClipboardAccess::Write),
            Trigger::Activation
        )),
        Denial::Revoked(key("clipboard:write"))
    );
}

#[test]
fn consent_approves_only_the_declaration_the_user_was_shown() {
    let (mut broker, package) = enabled(caps(json!({
        "http": [{ "origin": "https://api.github.com", "methods": ["GET"] }]
    })));
    let shown = consent_needed(check(&broker, &package, &get(API), Trigger::Activation));
    let _ = broker.install(
        package.clone(),
        rev(R2),
        caps(json!({
            "http": [{ "origin": "https://api.github.com", "methods": ["GET", "POST", "DELETE"] }]
        })),
    );
    assert_eq!(
        broker.consent(&package, &shown.revision, &shown.scope, None),
        Err(BrokerError::StaleRevision)
    );
    let _ = broker.install(
        package.clone(),
        rev(R1),
        caps(json!({
            "http": [{ "origin": "https://api.github.com", "methods": ["GET", "POST"] }]
        })),
    );
    assert_eq!(
        broker.consent(&package, &shown.revision, &shown.scope, None),
        Err(BrokerError::DeclarationChanged(key(
            "http:https://api.github.com"
        )))
    );
    assert_eq!(
        broker.consents(&package)[0].1,
        ConsentState::Undecided,
        "nothing wider was granted"
    );
}

#[test]
fn reloading_the_same_manifest_never_grants_and_keeps_the_generation() {
    let (mut broker, package) = enabled(github());
    let generation = broker.generation(&package);
    let report = broker.install(package.clone(), rev(R1), github());
    assert_eq!(report, Reconciliation::default());
    assert_eq!(broker.generation(&package), generation);
    assert_eq!(
        consent_needed(check(&broker, &package, &get(API), Trigger::Activation)).reason,
        ConsentReason::NotGranted
    );
}

#[test]
fn uninstalling_forgets_every_decision() {
    let (mut broker, package) = enabled(github());
    let scope = key("http:https://api.github.com");
    broker.approve(&package, &scope, None).unwrap();
    let old = permit(check(&broker, &package, &get(API), Trigger::Query));
    assert!(broker.uninstall(&package));
    assert!(!broker.validate(&old));
    assert_eq!(
        denial(check(&broker, &package, &get(API), Trigger::Query)),
        Denial::NotInstalled
    );
    assert_eq!(broker.generation(&package), None);
    assert_eq!(broker.revoke(&package, &scope), Err(BrokerError::NotInstalled));
    let _ = broker.install(package.clone(), rev(R1), github());
    assert!(!broker.is_enabled(&package));
    assert_eq!(broker.consents(&package)[0].1, ConsentState::Undecided);
    assert!(!broker.uninstall(&id("missing")));
}

#[test]
fn bundled_packages_start_on_with_extension_scopes_granted_but_os_access_still_asked() {
    let mut broker = Broker::new(Base::default(), Local::default());
    let package = id("calendar-tools");
    broker.set_bundled(package.clone(), rev(R1));
    let _ = broker.install(package.clone(), rev(R1), everything());
    assert!(broker.is_enabled(&package));
    let states: Vec<_> = broker
        .consents(&package)
        .into_iter()
        .map(|(scope, state)| (scope.key().to_string(), state))
        .collect();
    let bundled = ConsentState::Granted(GrantSource::Bundled);
    assert_eq!(
        states,
        [
            ("http:https://api.github.com".into(), bundled.clone()),
            ("files:notes".into(), ConsentState::Undecided),
            ("exec:git-status".into(), ConsentState::Undecided),
            ("script:open-in-zed".into(), bundled.clone()),
            ("storage".into(), bundled.clone()),
            ("clipboard:read".into(), bundled.clone()),
            ("clipboard:write".into(), bundled.clone()),
            ("clipboard:watch".into(), bundled.clone()),
            ("dictionary".into(), bundled.clone()),
            ("calendar".into(), bundled.clone()),
        ]
    );
    assert!(matches!(
        check(&broker, &package, &get(API), Trigger::Query),
        Decision::Allow(_)
    ));
    let read = Request::CalendarRead {
        past_days: 0,
        future_days: 1,
    };
    assert_eq!(
        consent_needed(check(&broker, &package, &read, Trigger::Activation)).reason,
        ConsentReason::OsAccess(OsPermission::Calendars)
    );
    assert!(matches!(
        denial(check(&broker, &package, &post(API), Trigger::Query)),
        Denial::NeedsActivation
    ));

    broker
        .revoke(&package, &key("http:https://api.github.com"))
        .unwrap();
    assert_eq!(
        denial(check(&broker, &package, &get(API), Trigger::Query)),
        Denial::Revoked(key("http:https://api.github.com"))
    );
    broker.set_enabled(&package, false).unwrap();
    assert!(!broker.is_enabled(&package));
}

#[test]
fn a_modified_copy_of_a_bundled_package_is_third_party() {
    let mut broker = Broker::new(Base::default(), Local::default());
    let package = id("calendar-tools");
    broker.set_bundled(package.clone(), rev(R1));
    let _ = broker.install(package.clone(), rev(R2), everything());
    assert!(!broker.is_enabled(&package));
    assert!(
        broker
            .consents(&package)
            .iter()
            .all(|(_, state)| *state == ConsentState::Undecided)
    );
    broker.set_enabled(&package, true).unwrap();
    assert_eq!(
        consent_needed(broker.check(&package, &rev(R2), &get(API), Trigger::Activation)).reason,
        ConsentReason::NotGranted
    );

    let restored = broker.install(package.clone(), rev(R1), everything());
    assert_eq!(restored, Reconciliation::default());
    assert!(matches!(
        check(&broker, &package, &get(API), Trigger::Query),
        Decision::Allow(_)
    ));
}

fn base(source: &str) -> Base {
    Base::parse(source).unwrap_or_else(|e| panic!("{e}"))
}

const DECLARED: &str = r#"
version = 1

[packages.bookmarks]
revision = "sha256:1111"
enabled = true
grant = ["http:https://api.github.com", "files:notes"]
deny = ["clipboard:read"]
bindings = { "files:notes" = "/Users/me/Notes" }
"#;

#[test]
fn the_declarative_base_grants_its_pinned_revision_and_local_choices_can_only_take_away() {
    let mut broker = Broker::new(base(DECLARED), Local::default());
    let package = id("bookmarks");
    let _ = broker.install(package.clone(), rev(R1), everything());
    assert!(broker.is_enabled(&package));
    let state = |broker: &Broker, scope: &str| {
        broker
            .consents(&package)
            .into_iter()
            .find(|(declared, _)| declared.key() == key(scope))
            .unwrap()
            .1
    };
    assert_eq!(
        state(&broker, "http:https://api.github.com"),
        ConsentState::Granted(GrantSource::Base)
    );
    assert_eq!(
        state(&broker, "files:notes"),
        ConsentState::Granted(GrantSource::Base)
    );
    assert_eq!(state(&broker, "clipboard:read"), ConsentState::DeniedByBase);
    assert_eq!(state(&broker, "dictionary"), ConsentState::Undecided);
    let files = Request::Files {
        scope: name("notes"),
        access: FilesAccess::List,
        path: String::new(),
    };
    assert_eq!(
        permit(check(&broker, &package, &files, Trigger::Query)).binding(),
        Some(std::path::Path::new("/Users/me/Notes"))
    );

    assert_eq!(
        broker.approve(&package, &key("clipboard:read"), None),
        Err(BrokerError::DeniedByBase(key("clipboard:read")))
    );
    broker.set_os_access(OsPermission::Pasteboard, OsAccess::Allowed);
    assert_eq!(
        denial(check(
            &broker,
            &package,
            &Request::Clipboard(ClipboardAccess::Read),
            Trigger::Activation
        )),
        Denial::DeniedByBase(key("clipboard:read"))
    );
    broker.approve(&package, &ScopeKey::Dictionary, None).unwrap();
    assert_eq!(
        state(&broker, "dictionary"),
        ConsentState::Granted(GrantSource::Local)
    );

    broker
        .revoke(&package, &key("http:https://api.github.com"))
        .unwrap();
    assert_eq!(
        state(&broker, "http:https://api.github.com"),
        ConsentState::Revoked
    );
    broker.set_enabled(&package, false).unwrap();
    assert!(!broker.is_enabled(&package));

    let _ = broker.install(package.clone(), rev(R2), everything());
    broker.set_enabled(&package, true).unwrap();
    assert_eq!(
        state(&broker, "files:notes"),
        ConsentState::Undecided,
        "base grants apply only to the revision they pin"
    );
    assert_eq!(
        state(&broker, "clipboard:read"),
        ConsentState::DeniedByBase,
        "denials apply to every revision"
    );
}

#[test]
fn a_base_that_disables_a_package_locks_it_off() {
    let mut broker = Broker::new(
        base("version = 1\n[packages.bookmarks]\nenabled = false\n"),
        Local::default(),
    );
    let package = id("bookmarks");
    broker.set_bundled(package.clone(), rev(R1));
    let _ = broker.install(package.clone(), rev(R1), github());
    assert!(!broker.is_enabled(&package));
    assert_eq!(
        broker.set_enabled(&package, true),
        Err(BrokerError::DisabledByBase)
    );
    assert_eq!(broker.set_enabled(&package, false), Ok(()));
}

#[test]
fn a_base_disable_pinned_to_one_revision_still_holds_for_the_next() {
    let mut broker = Broker::new(
        base("version = 1\n[packages.bookmarks]\nrevision = \"sha256:1111\"\nenabled = false\n"),
        Local::default(),
    );
    let package = id("bookmarks");
    broker.set_bundled(package.clone(), rev(R2));
    let _ = broker.install(package.clone(), rev(R2), github());
    assert!(!broker.is_enabled(&package));
    assert_eq!(
        broker.set_enabled(&package, true),
        Err(BrokerError::DisabledByBase)
    );
}

#[test]
fn rebinding_a_scope_the_base_bound_moves_to_a_new_generation() {
    let mut broker = Broker::new(base(DECLARED), Local::default());
    let package = id("bookmarks");
    let _ = broker.install(package.clone(), rev(R1), everything());
    let list = Request::Files {
        scope: name("notes"),
        access: FilesAccess::List,
        path: String::new(),
    };
    let listing = permit(check(&broker, &package, &list, Trigger::Query));
    let generation = broker.generation(&package);

    broker
        .approve(&package, &key("files:notes"), Some("/Users/me/Notes".into()))
        .unwrap();
    assert_eq!(
        broker.generation(&package),
        generation,
        "approving the same root takes nothing away"
    );
    broker
        .approve(&package, &key("files:notes"), Some("/Users/me/Other".into()))
        .unwrap();
    assert_ne!(broker.generation(&package), generation);
    assert!(!broker.validate(&listing));
}

#[test]
fn a_replaced_base_takes_authority_away_at_once_but_grants_nothing_until_relaunch() {
    let mut broker = Broker::new(base(DECLARED), Local::default());
    let package = id("bookmarks");
    let _ = broker.install(package.clone(), rev(R1), everything());
    let in_flight = permit(check(&broker, &package, &get(API), Trigger::Query));

    broker.replace_base(base(
        r#"
version = 1
[packages.bookmarks]
revision = "sha256:1111"
enabled = true
grant = ["dictionary"]
"#,
    ));
    assert!(!broker.validate(&in_flight));
    assert_eq!(
        consent_needed(check(&broker, &package, &get(API), Trigger::Activation)).reason,
        ConsentReason::NotGranted
    );
    assert_eq!(
        consent_needed(check(
            &broker,
            &package,
            &Request::DictionaryLookup,
            Trigger::Query
        ))
        .reason,
        ConsentReason::NotGranted,
        "a reload grants nothing new"
    );
    assert_eq!(
        denial(check(
            &broker,
            &package,
            &Request::Clipboard(ClipboardAccess::Read),
            Trigger::Activation
        )),
        Denial::DeniedByBase(key("clipboard:read")),
        "a denial from the launch-time base still holds"
    );

    let generation = broker.generation(&package);
    broker.replace_base(base(
        r#"
version = 1
[packages.bookmarks]
revision = "sha256:1111"
enabled = true
grant = ["dictionary"]
"#,
    ));
    assert_eq!(
        broker.generation(&package),
        generation,
        "an unchanged base drops nothing"
    );
}

#[test]
fn local_decisions_read_again_replace_those_in_memory() {
    let (mut broker, package) = enabled(github());
    broker
        .approve(&package, &key("http:https://api.github.com"), None)
        .unwrap();
    let in_flight = permit(check(&broker, &package, &get(API), Trigger::Query));
    let saved = Local::parse(&broker.local_toml().unwrap()).unwrap();
    assert!(
        broker.replace_local(saved).is_empty(),
        "reading back its own save changes nothing"
    );
    assert!(broker.validate(&in_flight));

    assert_eq!(
        broker.replace_local(Local::default()),
        std::slice::from_ref(&package)
    );
    assert!(!broker.validate(&in_flight));
    assert!(!broker.is_enabled(&package));
    assert!(
        !broker.local_toml().unwrap().contains("bookmarks"),
        "the next save keeps the removal"
    );

    assert_eq!(
        broker.replace_local(Local::unreadable()),
        std::slice::from_ref(&package)
    );
    assert_eq!(
        denial(check(&broker, &package, &get(API), Trigger::Activation)),
        Denial::GrantsUnreadable
    );
}

#[test]
fn unreadable_local_grants_fail_closed() {
    let mut broker = Broker::new(base(DECLARED), Local::unreadable());
    let package = id("bookmarks");
    broker.set_bundled(package.clone(), rev(R1));
    let _ = broker.install(package.clone(), rev(R1), everything());
    assert!(!broker.is_enabled(&package));
    assert_eq!(
        denial(check(&broker, &package, &get(API), Trigger::Activation)),
        Denial::GrantsUnreadable
    );
    assert_eq!(
        broker.set_enabled(&package, true),
        Err(BrokerError::GrantsUnreadable)
    );
    assert_eq!(
        broker.approve(&package, &ScopeKey::Dictionary, None),
        Err(BrokerError::GrantsUnreadable)
    );
    assert_eq!(
        broker.revoke(&package, &ScopeKey::Dictionary),
        Err(BrokerError::GrantsUnreadable)
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("grants.toml");
    std::fs::write(&path, "not toml").unwrap();
    assert!(matches!(broker.save(&path), Err(StoreError::Unreadable)));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "not toml");
}

#[test]
fn os_consumers_list_each_package_scope_that_needs_os_access() {
    let (mut broker, package) = enabled(everything());
    broker.approve(&package, &ScopeKey::Calendar, None).unwrap();
    let _ = broker.install(id("idle"), rev(R1), github());
    assert_eq!(
        broker.os_consumers(),
        [
            OsConsumer {
                permission: OsPermission::Pasteboard,
                package: package.clone(),
                scope: key("clipboard:read"),
                enabled: true,
                granted: false,
            },
            OsConsumer {
                permission: OsPermission::Pasteboard,
                package: package.clone(),
                scope: key("clipboard:watch"),
                enabled: true,
                granted: false,
            },
            OsConsumer {
                permission: OsPermission::Calendars,
                package: package.clone(),
                scope: ScopeKey::Calendar,
                enabled: true,
                granted: true,
            },
        ]
    );
}

#[test]
fn denials_explain_themselves() {
    assert_eq!(
        Denial::OsDenied(OsPermission::Calendars).to_string(),
        "macOS denied Calendars access for Dupin."
    );
    assert_eq!(
        Denial::Revoked(ScopeKey::Dictionary).to_string(),
        "You turned off `dictionary` for this extension."
    );
    assert_eq!(
        BrokerError::BindingRequired(key("files:notes")).to_string(),
        "`files:notes` needs a file, folder or program you pick"
    );
}

#[test]
fn only_a_choice_to_turn_a_package_off_counts_as_turned_off() {
    let (mut broker, package) = installed(github());
    assert!(!broker.is_enabled(&package));
    assert!(!broker.is_turned_off(&package));
    broker.set_enabled(&package, false).unwrap();
    assert!(broker.is_turned_off(&package));
    broker.set_enabled(&package, true).unwrap();
    assert!(!broker.is_turned_off(&package));
    assert!(broker.is_turned_off(&id("missing")));

    let base = Base::parse("version = 1\n[packages.bookmarks]\nenabled = false\n").unwrap();
    let mut locked = Broker::new(base, Local::default());
    let _ = locked.install(package.clone(), rev(R1), github());
    assert!(locked.is_turned_off(&package));

    let mut unreadable = Broker::new(Base::default(), Local::unreadable());
    let _ = unreadable.install(package.clone(), rev(R1), github());
    assert!(unreadable.is_turned_off(&package));
}

#[test]
fn a_change_that_cannot_be_saved_is_undone() {
    let (mut broker, package) = enabled(github());
    let dir = tempfile::tempdir().unwrap();
    let blocked = dir.path().join("file");
    std::fs::write(&blocked, "").unwrap();
    let unwritable = blocked.join("grants.toml");

    let failed = broker.change_saved(&unwritable, |broker| broker.set_enabled(&package, false));
    assert!(
        matches!(failed, Err(SaveError::Store(StoreError::Io(_)))),
        "{failed:?}"
    );
    assert!(broker.is_enabled(&package));
    assert!(!broker.is_turned_off(&package));

    let refused = broker.change_saved(&dir.path().join("grants.toml"), |broker| {
        broker.set_enabled(&id("missing"), false)
    });
    assert!(matches!(
        refused,
        Err(SaveError::Refused(BrokerError::NotInstalled))
    ));
    assert!(!dir.path().join("grants.toml").exists());

    let path = dir.path().join("grants.toml");
    broker
        .change_saved(&path, |broker| broker.set_enabled(&package, false))
        .unwrap();
    assert!(broker.is_turned_off(&package));
    let saved = Local::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut relaunched = Broker::new(Base::default(), saved);
    let _ = relaunched.install(package.clone(), rev(R1), github());
    assert!(relaunched.is_turned_off(&package));
}
