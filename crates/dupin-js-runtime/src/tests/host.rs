use std::collections::{HashMap, VecDeque};

use super::*;
use crate::row::RowEffect;

/// Answers each request with the reply set for its operation, or a denial, and records what it
/// was sent.
#[derive(Default)]
struct Fixture {
    replies: HashMap<&'static str, String>,
    sent: Vec<(String, serde_json::Value)>,
    queued: VecDeque<(u32, String)>,
    /// Never answers, as a host whose service hangs.
    silent: bool,
}

impl Bridge for Fixture {
    fn send(&mut self, call: u32, op: &str, args: &str) -> Result<(), Diagnostic> {
        self.sent.push((op.into(), serde_json::from_str(args).unwrap()));
        self.queued.push_back((call, op.into()));
        Ok(())
    }

    fn receive(&mut self, stop: &dyn Fn() -> bool) -> Option<Reply> {
        if self.silent {
            while !stop() {
                std::thread::sleep(Duration::from_millis(1));
            }
            return None;
        }
        let (call, op) = self.queued.pop_front()?;
        let json = self
            .replies
            .get(op.as_str())
            .cloned()
            .unwrap_or_else(|| r#"{"error":{"code":"denied","message":"no"}}"#.into());
        Some(Reply { call, json })
    }
}

fn with_host(source: &str, query: &str, bridge: &mut Fixture) -> Result<Vec<Row>, RunError> {
    let (_folder, loaded) = package(&[("main.js", source)]);
    loaded
        .unwrap()
        .search_with(&engine(), query, MAX_ROWS, timeout(DEADLINE), bridge)
}

fn titles(rows: &[Row]) -> Vec<String> {
    rows.iter().map(|row| row.title.clone()).collect()
}

fn failure_of<T: std::fmt::Debug>(result: Result<T, RunError>) -> Diagnostic {
    match result {
        Err(RunError::Failed(diagnostic)) => diagnostic,
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn host_requests_reach_the_bridge_and_replies_settle_their_promises() {
    let mut bridge = Fixture::default();
    bridge.replies.insert(
        "http.fetch",
        r#"{"ok":{"status":200,"url":"https://api.example.com/x","redirected":false,"headers":{"content-type":"application/json"},"body":"{\"archived\":false}","truncated":false}}"#.into(),
    );
    bridge.replies.insert(
        "storage.get",
        r#"{"ok":{"id":"r","createdAt":0,"data":{"n":3},"blobs":[]}}"#.into(),
    );
    let rows = with_host(
        r#"import { http, storage } from 'dupin';
           export async function search(query) {
             const [response, stored] = await Promise.all([
               http.fetch('https://api.example.com/x', { headers: { accept: 'application/json' } }),
               storage.get(query),
             ]);
             const repo = await response.json();
             return [{ title: `${response.ok} ${response.status} ${repo.archived} ${stored.data.n}` }];
           }"#,
        "count",
        &mut bridge,
    )
    .unwrap();
    assert_eq!(titles(&rows), ["true 200 false 3"]);
    assert_eq!(
        bridge.sent,
        [
            (
                "http.fetch".to_owned(),
                serde_json::json!({"url": "https://api.example.com/x", "method": "GET", "headers": {"accept": "application/json"}})
            ),
            ("storage.get".to_owned(), serde_json::json!({"id": "count"})),
        ]
    );
}

#[test]
fn blobs_cross_as_base64_and_come_back_as_bytes() {
    let mut bridge = Fixture::default();
    bridge.replies.insert(
        "storage.put",
        r#"{"ok":{"id":"r","createdAt":86400000,"data":"x","blobs":[{"handle":"h","size":4}]}}"#.into(),
    );
    bridge
        .replies
        .insert("storage.readBlob", r#"{"ok":{"data":"AAEC/w=="}}"#.into());
    let rows = with_host(
        r#"import { storage } from 'dupin';
           export async function search() {
             const long = new Uint8Array(70001).map((_, index) => index % 251);
             const record = await storage.put('x', { blobs: [new Uint8Array([0, 1, 2, 255]), new Uint8Array([1]).buffer, new Uint16Array([0x4241]), long, new Uint8Array(0)] });
             const bytes = await storage.readBlob(record.blobs[0].handle);
             return [{ title: `${record.createdAt} ${Object.isFrozen(record.blobs[0])} ${bytes instanceof Uint8Array} ${[...bytes]}` }];
           }"#,
        "",
        &mut bridge,
    )
    .unwrap();
    assert_eq!(titles(&rows), ["1970-01-02T00:00:00.000Z true true 0,1,2,255"]);
    let blobs = bridge.sent[0].1["blobs"].as_array().unwrap();
    assert_eq!(blobs[..3], ["AAEC/w==", "AQ==", "QUI="]);
    let long: Vec<u8> = (0..70001).map(|index| (index % 251) as u8).collect();
    assert_eq!(
        base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            blobs[3].as_str().unwrap()
        )
        .unwrap(),
        long
    );
    assert_eq!(blobs[4], "");
    assert_eq!(
        bridge.sent[1],
        ("storage.readBlob".to_owned(), serde_json::json!({"handle": "h"}))
    );
}

#[test]
fn a_failed_request_rejects_with_a_dupin_error_the_guest_can_read() {
    let mut bridge = Fixture::default();
    bridge.replies.insert(
        "clipboard.read",
        r#"{"error":{"code":"needs-consent","message":"Allow clipboard reading first.","scope":"clipboard:read"}}"#
            .into(),
    );
    let rows = with_host(
        r#"import { clipboard, DupinError } from 'dupin';
           export async function search() {
             try { await clipboard.readText(); return []; }
             catch (error) { return [{ title: `${error instanceof DupinError} ${error.code} ${error.scope}`, subtitle: error.message }]; }
           }"#,
        "",
        &mut bridge,
    )
    .unwrap();
    assert_eq!(titles(&rows), ["true needs-consent clipboard:read"]);

    let diagnostic = failure(with_host(
        "import { files } from 'dupin'; export const search = async () => [{ title: await files.readText('notes', 'a.md') }];",
        "",
        &mut Fixture::default(),
    ));
    assert_eq!(diagnostic.code, "search-threw");
    assert!(diagnostic.message.contains("DupinError: no"), "{diagnostic:?}");
}

#[test]
fn a_call_can_have_only_so_many_requests_waiting_or_made() {
    let mut bridge = Fixture::default();
    bridge
        .replies
        .insert("dictionary.lookup", r#"{"ok":{"entries":[]}}"#.into());
    let rows = with_host(
        r#"import { dictionary } from 'dupin';
           export async function search() {
             const results = await Promise.allSettled(Array.from({ length: 20 }, (_, i) => dictionary.lookup(`w${i}`)));
             const refused = results.filter((r) => r.status === 'rejected' && r.reason.code === 'too-many-calls').length;
             let made = 0;
             for (let i = 0; i < 300; i++) {
               try { await dictionary.lookup('x'); made++; } catch (error) { if (error.code !== 'too-many-calls') throw error; }
             }
             return [{ title: `${refused} ${made}` }];
           }"#,
        "",
        &mut bridge,
    )
    .unwrap();
    // Sixteen may wait at once, and the first batch used sixteen of the call's 256 requests.
    assert_eq!(titles(&rows), ["4 240"]);
    assert_eq!(bridge.sent.len(), MAX_CALLS as usize);
}

#[test]
fn a_request_nobody_answers_ends_at_the_deadline_and_cancelling_ends_it_quietly() {
    let source = "import { http } from 'dupin'; export async function search() { await http.fetch('https://example.com'); return []; }";
    let (_folder, loaded) = package(&[("main.js", source)]);
    let package = loaded.unwrap();
    let started = Instant::now();
    let mut silent = Fixture {
        silent: true,
        ..Fixture::default()
    };
    let diagnostic = failure(package.search_with(
        &engine(),
        "",
        MAX_ROWS,
        timeout(Duration::from_millis(100)),
        &mut silent,
    ));
    assert_eq!(diagnostic.code, "deadline");
    assert!(started.elapsed() < Duration::from_secs(1));

    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        flag.store(true, Ordering::Relaxed);
    });
    let control = Control {
        deadline: (Instant::now() + DEADLINE).into(),
        is_cancelled: Box::new(move || cancelled.load(Ordering::Relaxed)),
    };
    assert_eq!(
        package.search_with(&engine(), "", MAX_ROWS, control, &mut silent),
        Err(RunError::Cancelled)
    );
}

#[test]
fn without_a_host_every_request_fails_as_unavailable() {
    let rows = run(
        "import { storage } from 'dupin'; export async function search() { try { await storage.usage(); } catch (error) { return [{ title: error.code }]; } }",
        "",
    )
    .unwrap();
    assert_eq!(titles(&rows), ["unavailable"]);
}

#[test]
fn items_offer_named_actions_with_bounded_data() {
    let rows = run(
        r#"export const search = () => [{ title: 'Note', text: 'body', actions: [{ title: 'Save', run: 'save', data: { id: 7 } }, { title: 'Pin', run: 'pin' }] }];"#,
        "",
    )
    .unwrap();
    let effects: Vec<_> = rows[0].actions.iter().map(|action| &action.effect).collect();
    assert_eq!(
        effects,
        [
            &RowEffect::Copy { text: "body".into() },
            &RowEffect::Run {
                handler: "save".into(),
                data: Some(serde_json::json!({"id": 7}))
            },
            &RowEffect::Run {
                handler: "pin".into(),
                data: None
            },
        ]
    );
    for (actions, field) in [
        ("{ title: 'x' }", "items[0].actions[0]"),
        ("{ title: 'x', run: 'y', go: 1 }", "items[0].actions[0].go"),
        (
            "{ title: 'x', run: 'y', data: 'z'.repeat(5000) }",
            "items[0].actions[0].data",
        ),
        (
            "{ title: 'x', run: 'y', data: JSON.parse('['.repeat(20) + ']'.repeat(20)) }",
            "items[0].actions[0].data",
        ),
        ("...Array(9).fill({ title: 'x', run: 'y' })", "items[0].actions"),
    ] {
        let diagnostic = failure(run(
            &format!("export const search = () => [{{ title: 'Note', actions: [{actions}] }}];"),
            "",
        ));
        assert_eq!(
            diagnostic.field.as_deref(),
            Some(field),
            "{actions}: {diagnostic:?}"
        );
    }
}

fn act(source: &str, name: &str, data: &str, bridge: &mut Fixture) -> Result<Option<ActionEffect>, RunError> {
    let (_folder, loaded) = package(&[("main.js", source)]);
    loaded
        .unwrap()
        .action(
            &engine(),
            name,
            data,
            &crate::PageContext::default(),
            &serde_json::json!({}),
            timeout(DEADLINE),
            bridge,
        )
        .map(|acted| acted.effect)
}

#[test]
fn actions_run_by_name_with_their_data_and_return_an_effect() {
    let mut bridge = Fixture::default();
    bridge.replies.insert(
        "storage.put",
        r#"{"ok":{"id":"r","createdAt":0,"data":null,"blobs":[]}}"#.into(),
    );
    let source = r#"import { storage } from 'dupin';
        export function search() { return []; }
        export const actions = {
          async save(data, ctx) { await storage.put(data); return { type: 'toast', text: `Saved ${data.id} ${Object.isFrozen(ctx)}` }; },
          quiet() {},
          bad() { return { type: 'toast', text: 'a\nb' }; },
        };"#;
    assert_eq!(
        act(source, "save", r#"{"id":7}"#, &mut bridge),
        Ok(Some(ActionEffect::Toast {
            text: "Saved 7 true".into()
        }))
    );
    assert_eq!(
        bridge.sent,
        [(
            "storage.put".to_owned(),
            serde_json::json!({"data": {"id": 7}, "blobs": []})
        )]
    );
    assert_eq!(act(source, "quiet", "null", &mut bridge), Ok(None));
    assert_eq!(
        failure_of(act(source, "bad", "null", &mut bridge)).code,
        "invalid-effect"
    );
    for missing in ["nope", "toString", "__proto__", "constructor", ""] {
        assert_eq!(
            failure_of(act(source, missing, "null", &mut bridge)).code,
            "missing-handler",
            "{missing}"
        );
    }
    let thrown = failure_of(act(
        "export const actions = { boom() { throw new Error('x'); } };",
        "boom",
        "null",
        &mut bridge,
    ));
    assert_eq!(thrown.code, "action-threw");
}

#[test]
fn items_reaches_host_services_like_search() {
    let mut bridge = Fixture::default();
    bridge.replies.insert(
        "storage.list",
        r#"{"ok":{"records":[{"id":"r1","createdAt":0,"data":"copied","blobs":[]}],"next":null}}"#.into(),
    );
    let (_folder, loaded) = package(&[(
        "main.js",
        r#"import { storage } from 'dupin';
           export async function items() {
             const { records } = await storage.list({ limit: 10 });
             return records.map((record) => ({ id: record.id, title: record.data }));
           }"#,
    )]);
    let page = loaded
        .unwrap()
        .items(
            &engine(),
            None,
            Reason::Load,
            &serde_json::json!({}),
            timeout(DEADLINE),
            &mut bridge,
        )
        .unwrap();
    let crate::row::PageChange::Reset { rows } = page.change else {
        panic!("expected the whole catalogue, got {:?}", page.change);
    };
    assert_eq!(
        rows.iter().map(|row| row.title.as_str()).collect::<Vec<_>>(),
        ["copied"]
    );
    assert_eq!(
        bridge.sent,
        [("storage.list".to_owned(), serde_json::json!({"limit": 10}))]
    );
}

#[test]
fn an_event_handler_may_invalidate_the_catalogue() {
    let (_folder, loaded) = package(&[(
        "main.js",
        r#"import { items as catalogue } from 'dupin';
           export function items() { return []; }
           export const events = { clipboard(change) { if (change.text) catalogue.invalidate(); } };"#,
    )]);
    let package = loaded.unwrap();
    let mut bridge = Fixture::default();
    let mut handle = |event: &str| {
        package
            .event(
                &engine(),
                "clipboard",
                event,
                &serde_json::json!({}),
                timeout(DEADLINE),
                &mut bridge,
            )
            .unwrap()
    };
    assert!(handle(r#"{"type":"text","text":"hi"}"#).invalidated);
    assert!(!handle(r#"{"type":"text","text":""}"#).invalidated);
}

#[test]
fn event_handlers_receive_a_frozen_event() {
    let mut bridge = Fixture::default();
    bridge.replies.insert(
        "storage.put",
        r#"{"ok":{"id":"r","createdAt":0,"data":null,"blobs":[]}}"#.into(),
    );
    let (_folder, loaded) = package(&[(
        "main.js",
        r#"import { storage } from 'dupin';
           export const events = { async clipboard(change) { await storage.put(`${change.text} ${Object.isFrozen(change)}`); } };"#,
    )]);
    let package = loaded.unwrap();
    package
        .event(
            &engine(),
            "clipboard",
            r#"{"type":"text","text":"hi"}"#,
            &serde_json::json!({}),
            timeout(DEADLINE),
            &mut bridge,
        )
        .unwrap();
    assert_eq!(
        bridge.sent,
        [(
            "storage.put".to_owned(),
            serde_json::json!({"data": "hi true", "blobs": []})
        )]
    );
    assert_eq!(
        failure_of(package.event(
            &engine(),
            "other",
            "{}",
            &serde_json::json!({}),
            timeout(DEADLINE),
            &mut bridge
        ))
        .code,
        "missing-handler"
    );
}

#[test]
fn a_filter_sees_only_frozen_titles_and_reaches_no_host_service() {
    let mut bridge = Fixture::default();
    bridge.replies.insert(
        "storage.put",
        r#"{"ok":{"id":"r","createdAt":0,"data":null,"blobs":[]}}"#.into(),
    );
    let (_folder, loaded) = package(&[(
        "main.js",
        r#"import { storage } from 'dupin';
           export const filters = {
             async romaji(input, ctx) {
               let leaked = 'kept';
               try { await storage.put(input.query); } catch (error) { leaked = error.code; }
               const frozen = Object.isFrozen(input) && Object.isFrozen(input.candidates) && Object.isFrozen(input.candidates[0]);
               return input.candidates
                 .map((candidate, index) => ({ index, score: candidate.title.length, keys: Object.keys(candidate).join() }))
                 .filter((found) => frozen && leaked === 'unavailable' && found.keys === 'title' && ctx.preferences.on)
                 .map(({ index, score }) => ({ index, score, positions: [0] }));
             },
             reverse(input) { return input.candidates.map((_, index) => input.candidates.length - 1 - index); },
             twice() { return [0, 0]; },
             query(input) { return 'query' in input ? [] : [0, 1]; },
           };"#,
    )]);
    let package = loaded.unwrap();
    let mut run = |name: &str, stage| {
        let input = FilterInput::new(stage, Some("sa".into()), ["Safari".into(), "Sa".into()]);
        package.filter(
            &engine(),
            name,
            &input,
            &serde_json::json!({ "on": true }),
            timeout(DEADLINE),
            &mut bridge,
        )
    };
    let hits = run("romaji", FilterStage::Matcher).unwrap();
    assert_eq!(
        hits.iter().map(|hit| (hit.index, hit.score)).collect::<Vec<_>>(),
        [(0, Some(6)), (1, Some(2))]
    );
    let order: Vec<_> = run("reverse", FilterStage::Sorter)
        .unwrap()
        .iter()
        .map(|hit| hit.index)
        .collect();
    assert_eq!(order, [1, 0]);
    assert_eq!(
        failure_of(run("twice", FilterStage::Sorter)).code,
        "invalid-filter-answer"
    );
    assert_eq!(
        run("query", FilterStage::Sorter).unwrap().len(),
        2,
        "a sorter is never handed the query"
    );
    assert_eq!(
        failure_of(run("missing", FilterStage::Matcher)).code,
        "missing-handler"
    );
    assert!(bridge.sent.is_empty(), "{:?}", bridge.sent);
}

#[test]
fn a_filter_that_never_returns_is_stopped_at_its_deadline() {
    let (_folder, loaded) = package(&[("main.js", "export const filters = { spin() { for (;;) {} } };")]);
    let input = FilterInput::new(FilterStage::Sorter, None, ["a".into()]);
    let diagnostic = failure_of(loaded.unwrap().filter(
        &engine(),
        "spin",
        &input,
        &serde_json::json!({}),
        timeout(Duration::from_millis(50)),
        &mut Fixture::default(),
    ));
    assert_eq!(diagnostic.code, "deadline");
}
