use std::sync::Mutex;

use super::*;

/// Answers requests by operation after `delay`, and records each request and whether its call
/// had ended by the time the answer was ready.
#[derive(Default)]
struct Fixture {
    delay: Duration,
    seen: Mutex<Vec<(String, serde_json::Value)>>,
    late: AtomicUsize,
}

impl Services for Fixture {
    fn call(&self, op: &str, args: &[u8], _deadline: Instant, cancelled: &AtomicBool) -> Vec<u8> {
        let args: serde_json::Value = serde_json::from_slice(args).unwrap();
        self.seen.lock().unwrap().push((op.into(), args.clone()));
        std::thread::sleep(self.delay);
        if cancelled.load(Ordering::Relaxed) {
            self.late.fetch_add(1, Ordering::Relaxed);
        }
        let reply = match op {
            "storage.get" => {
                serde_json::json!({ "ok": { "id": args["id"], "createdAt": 0, "data": format!("stored {}", args["id"]), "blobs": [] } })
            }
            _ => {
                serde_json::json!({ "error": { "code": "denied", "message": format!("{op} is not allowed") } })
            }
        };
        reply.to_string().into_bytes()
    }
}

fn services(fixture: &Arc<Fixture>) -> Arc<dyn Services> {
    fixture.clone()
}

#[test]
fn host_requests_cross_to_dupin_and_back_during_a_search() {
    skip_inside_a_sandbox!();
    let host = host();
    let package = package(
        r#"import { storage, clipboard } from 'dupin';
           export async function search(query) {
             const { data: value } = await storage.get(query);
             let denied = '';
             try { await clipboard.readText(); } catch (error) { denied = `${error.code}: ${error.message}`; }
             return [{ title: value, subtitle: denied }];
           }"#,
    );
    let fixture = Arc::new(Fixture::default());
    let rows = host
        .search(
            &package,
            "count",
            10,
            Instant::now() + Duration::from_secs(5),
            &|| false,
            &services(&fixture),
        )
        .unwrap();
    assert_eq!(titles(&rows), ["stored \"count\""]);
    assert_eq!(rows[0].subtitle, "denied: clipboard.read is not allowed");
    assert_eq!(
        *fixture.seen.lock().unwrap(),
        [
            ("storage.get".to_owned(), serde_json::json!({"id": "count"})),
            ("clipboard.read".to_owned(), serde_json::json!({})),
        ]
    );
}

#[test]
fn a_catalogue_reaches_host_services_as_a_search_does() {
    skip_inside_a_sandbox!();
    let host = host();
    let package = package(
        r#"import { storage } from 'dupin';
           export async function items(ctx) {
             const { data } = await storage.get(ctx.reason);
             return [{ id: 'stored', title: data }];
           }"#,
    );
    let fixture = Arc::new(Fixture::default());
    let page = host
        .items(
            &package,
            None,
            Reason::Load,
            serde_json::json!({}),
            Instant::now() + Duration::from_secs(5),
            &|| false,
            &services(&fixture),
        )
        .unwrap();
    let dupin_js_runtime::row::PageChange::Reset { rows } = page.change else {
        panic!("{:?}", page.change);
    };
    assert_eq!(titles(&rows), ["stored \"load\""]);
    assert_eq!(
        *fixture.seen.lock().unwrap(),
        [("storage.get".to_owned(), serde_json::json!({"id": "load"}))]
    );
}

#[test]
fn a_finished_request_reaches_the_guest_at_once() {
    skip_inside_a_sandbox!();
    let host = host();
    let package = package(
        r#"import { storage } from 'dupin';
           export async function search(query) {
             for (let i = 0; i < 100; i++) await storage.get(query);
             return [{ title: 'done' }];
           }"#,
    );
    let fixture = Arc::new(Fixture::default());
    let run = || {
        host.search(
            &package,
            "count",
            10,
            Instant::now() + Duration::from_secs(5),
            &|| false,
            &services(&fixture),
        )
        .unwrap()
    };
    run();
    let started = Instant::now();
    assert_eq!(titles(&run()), ["done"]);
    // Waiting out the host's 5 ms check for each reply would take a quarter of a second.
    assert!(
        started.elapsed() < Duration::from_millis(150),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn actions_and_events_run_in_the_helper_with_their_data() {
    skip_inside_a_sandbox!();
    let host = host();
    let package = package(
        r#"import { storage } from 'dupin';
           export const search = () => [];
           export const actions = { async show(data) { return { type: 'toast', text: `${(await storage.get(data.key)).data}` }; } };
           export const events = { async clipboard(change) { await storage.get(change.text); } };"#,
    );
    let fixture = Arc::new(Fixture::default());
    let acted = host
        .action(
            &package,
            "show",
            &serde_json::json!({"key": "k"}),
            &serde_json::json!({}),
            Instant::now() + Duration::from_secs(5),
            &|| false,
            &services(&fixture),
        )
        .unwrap();
    assert_eq!(
        acted.effect,
        Some(ActionEffect::Toast {
            text: "stored \"k\"".into()
        })
    );
    host.event(
        &package,
        "clipboard",
        &serde_json::json!({"type": "text", "text": "copied"}),
        &serde_json::json!({}),
        Instant::now() + Duration::from_secs(5),
        &|| false,
        &services(&fixture),
    )
    .unwrap();
    assert_eq!(
        fixture.seen.lock().unwrap().last().unwrap(),
        &("storage.get".to_owned(), serde_json::json!({"id": "copied"}))
    );
    let missing = failed(host.action(
        &package,
        "absent",
        &serde_json::Value::Null,
        &serde_json::json!({}),
        Instant::now() + Duration::from_secs(5),
        &|| false,
        &services(&fixture),
    ));
    assert_eq!(missing.code, "missing-handler");
}

#[test]
fn a_service_slower_than_the_call_has_its_late_reply_dropped() {
    skip_inside_a_sandbox!();
    let host = host();
    let package = package(
        "import { storage } from 'dupin'; export async function search() { return [{ title: (await storage.get('x')).data }]; }",
    );
    let fixture = Arc::new(Fixture {
        delay: Duration::from_millis(400),
        ..Fixture::default()
    });
    let started = Instant::now();
    let result = host.search(
        &package,
        "",
        10,
        Instant::now() + Duration::from_millis(100),
        &|| false,
        &services(&fixture),
    );
    assert_eq!(failed(result).code, "deadline");
    assert!(started.elapsed() < Duration::from_millis(400));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(fixture.late.load(Ordering::Relaxed), 1, "the call had ended");

    // The helper that gave up on the request still answers the next call.
    let quick = Arc::new(Fixture::default());
    let rows = host
        .search(
            &package,
            "",
            10,
            Instant::now() + Duration::from_secs(5),
            &|| false,
            &services(&quick),
        )
        .unwrap();
    assert_eq!(titles(&rows), ["stored \"x\""]);
}

#[test]
fn actions_and_events_waiting_on_a_service_never_hold_up_searches() {
    skip_inside_a_sandbox!();
    let host = Arc::new(host());
    let package = package(
        r#"import { storage } from 'dupin';
           export const search = (query) => [{ title: query }];
           export const actions = { async wait() { await storage.get('slow'); } };
           export const events = { async clipboard() { await storage.get('slow'); } };"#,
    );
    let slow = Arc::new(Fixture {
        delay: Duration::from_secs(3),
        ..Fixture::default()
    });
    let background: Vec<_> = [true, false]
        .into_iter()
        .map(|action| {
            let (host, package, slow) = (host.clone(), package.clone(), services(&slow));
            std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(10);
                let none = serde_json::Value::Null;
                if action {
                    host.action(&package, "wait", &none, &none, deadline, &|| false, &slow)
                        .map(drop)
                } else {
                    host.event(&package, "clipboard", &none, &none, deadline, &|| false, &slow)
                        .map(drop)
                }
            })
        })
        .collect();
    while slow.seen.lock().unwrap().len() < 2 {
        std::thread::sleep(Duration::from_millis(10));
    }
    let started = Instant::now();
    let rows = host
        .search(
            &package,
            "typed",
            10,
            Instant::now() + Duration::from_secs(2),
            &|| false,
            &services(&Arc::new(Fixture::default())),
        )
        .unwrap();
    assert_eq!(titles(&rows), ["typed"]);
    assert!(started.elapsed() < Duration::from_secs(2));
    for call in background {
        call.join().unwrap().unwrap();
    }
}

/// Gave the guest one calendar link.
struct Issued;

const ISSUED: &str = "ical://ekevent/issued?method=show&options=more";

impl Services for Issued {
    fn call(&self, _op: &str, _args: &[u8], _deadline: Instant, _cancelled: &AtomicBool) -> Vec<u8> {
        br#"{"ok":null}"#.to_vec()
    }

    fn opens(&self, url: &str) -> bool {
        url == ISSUED
    }
}

#[test]
fn a_row_opens_only_the_calendar_links_its_own_call_was_given() {
    skip_inside_a_sandbox!();
    let host = host();
    let package = package(
        r#"export function search(query) {
             return [{ title: 'Event', actions: [{ title: 'Open in Calendar', open: `ical://ekevent/${query}?method=show&options=more` }] }];
           }"#,
    );
    let search = |query: &str, services: Arc<dyn Services>| {
        host.search(
            &package,
            query,
            10,
            Instant::now() + Duration::from_secs(5),
            &|| false,
            &services,
        )
    };
    assert_eq!(search("issued", Arc::new(Issued)).unwrap().len(), 1);
    for (query, services) in [
        ("forged", Arc::new(Issued) as Arc<dyn Services>),
        ("issued", Arc::new(NoServices)),
    ] {
        let Err(RunError::Failed(diagnostic)) = search(query, services) else {
            panic!("opened a calendar link its call wasn't given: {query}");
        };
        assert_eq!(diagnostic.code, "invalid-url");
    }
}

#[test]
fn a_row_opens_a_dictionary_word_no_reply_gave() {
    skip_inside_a_sandbox!();
    let package = package(
        r#"export function search(query) {
             return [{ title: query, actions: [{ title: 'Open in Dictionary', open: `dict://${query}` }] }];
           }"#,
    );
    let services: Arc<dyn Services> = Arc::new(NoServices);
    let rows = host()
        .search(
            &package,
            "serendipity",
            10,
            Instant::now() + Duration::from_secs(5),
            &|| false,
            &services,
        )
        .unwrap();
    assert_eq!(titles(&rows), ["serendipity"]);
}
