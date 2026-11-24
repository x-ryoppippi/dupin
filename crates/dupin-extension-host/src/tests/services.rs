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
            "storage.get" => serde_json::json!({ "ok": { "value": format!("stored {}", args["key"]) } }),
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
             const value = await storage.get(query);
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
    let View::Text { subtitle, .. } = &rows[0].view else {
        unreachable!()
    };
    assert_eq!(subtitle, "denied: clipboard.read is not allowed");
    assert_eq!(
        *fixture.seen.lock().unwrap(),
        [
            ("storage.get".to_owned(), serde_json::json!({"key": "count"})),
            ("clipboard.read".to_owned(), serde_json::json!({})),
        ]
    );
}

#[test]
fn actions_and_events_run_in_the_helper_with_their_data() {
    skip_inside_a_sandbox!();
    let host = host();
    let package = package(
        r#"import { storage } from 'dupin';
           export const search = () => [];
           export const actions = { async show(data) { return { type: 'toast', text: `${await storage.get(data.key)}` }; } };
           export const events = { async clipboard(change) { await storage.get(change.text); } };"#,
    );
    let fixture = Arc::new(Fixture::default());
    let effect = host
        .action(
            &package,
            "show",
            &serde_json::json!({"key": "k"}),
            Instant::now() + Duration::from_secs(5),
            &|| false,
            &services(&fixture),
        )
        .unwrap();
    assert_eq!(
        effect,
        Some(ActionEffect::Toast {
            text: "stored \"k\"".into()
        })
    );
    host.event(
        &package,
        "clipboard",
        &serde_json::json!({"type": "text", "text": "copied"}),
        Instant::now() + Duration::from_secs(5),
        &|| false,
        &services(&fixture),
    )
    .unwrap();
    assert_eq!(
        fixture.seen.lock().unwrap().last().unwrap(),
        &("storage.get".to_owned(), serde_json::json!({"key": "copied"}))
    );
    let missing = failed(host.action(
        &package,
        "absent",
        &serde_json::Value::Null,
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
        "import { storage } from 'dupin'; export async function search() { return [{ title: await storage.get('x') }]; }",
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
