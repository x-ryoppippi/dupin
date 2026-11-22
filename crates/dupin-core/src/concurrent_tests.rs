use crate::session::Results;
use crate::*;
use std::sync::{Arc, Mutex, mpsc};

fn id(s: &str) -> CommandId {
    CommandId::new(s).unwrap()
}
fn registry(pin: bool) -> CommandRegistry {
    let mut r = CommandRegistry::new(id("apps.search"), Some(id("slow.search")));
    for name in ["apps.search", "slow.search"] {
        r.register(CommandSpec::new(id(name), 3, QueryPresentation::new("Search")))
            .unwrap();
    }
    r.add_keyword(&id("slow.search"), "slow").unwrap();
    r.set_default_providers(
        vec![id("apps.search"), id("slow.search")],
        pin.then(|| id("slow.search")),
    )
    .unwrap();
    r
}
fn fallback_registry() -> CommandRegistry {
    let apps = id("apps.search");
    let keywords = id("web.keywords");
    let fallbacks = id("web.fallbacks");
    let mut commands = CommandRegistry::new(apps.clone(), None);
    commands
        .register(CommandSpec::new(
            apps.clone(),
            2,
            QueryPresentation::new("Search"),
        ))
        .unwrap();
    commands
        .register(CommandSpec::new(
            keywords.clone(),
            5,
            QueryPresentation::new("Keywords"),
        ))
        .unwrap();
    commands
        .register(CommandSpec::new(fallbacks.clone(), 5, QueryPresentation::new("Fallbacks")).with_fallback())
        .unwrap();
    commands
        .set_default_providers(vec![apps, keywords.clone(), fallbacks], Some(keywords))
        .unwrap();
    commands
}
fn item(name: &str) -> Item {
    Item::path(name.into(), name.into(), vec![])
}
fn batch(generation: u64, names: &[&str]) -> Delivery {
    Delivery::Batch {
        generation,
        items: names.iter().map(|s| item(s)).collect(),
    }
}
fn visible_ids(session: &Session) -> Vec<&str> {
    session.items().iter().map(|item| item.id.as_str()).collect()
}
fn start(s: &mut Session, query: &str) -> u64 {
    s.set_query(query)
        .iter()
        .find_map(|effect| match effect {
            Effect::Search { generation, .. } => Some(*generation),
            _ => None,
        })
        .expect("query starts a search")
}

#[test]
fn expiry_preserves_delivered_rows_and_rejects_late_updates() {
    let mut s = Session::new(History::default(), registry(false)).unwrap();
    let g = start(&mut s, "slow term");
    s.deliver_from(id("slow.search"), batch(g, &["valid"]));
    let key = s.result_key(0);
    s.deliver_from(id("slow.search"), Delivery::Expired { generation: g });
    assert_eq!(s.results(), Results::Done);
    assert_eq!(s.items()[0].id, "valid");
    assert_eq!(s.failures().count(), 1);
    assert!(!s.deliver_from(id("slow.search"), batch(g, &["late"])));
    assert_eq!(
        s.run_key_action(&key, "copy-path")[0],
        Effect::Run(Action::Copy("valid".into()))
    );
}

#[test]
fn empty_completion_clears_previous_query_without_waiting_for_other_provider() {
    let mut s = Session::new(History::default(), registry(false)).unwrap();
    let g = start(&mut s, "old");
    s.deliver_from(id("apps.search"), batch(g, &["previous"]));
    let key = s.result_key(0);
    let g = start(&mut s, "new");
    assert_eq!(s.items()[0].id, "previous");
    s.deliver_from(id("apps.search"), Delivery::Done { generation: g });
    assert!(s.items().is_empty());
    assert!(s.pending(&id("slow.search"), g));
    assert!(s.run_key_action(&key, "open").is_empty());
    s.deliver_from(id("slow.search"), batch(g, &["current"]));
    assert_eq!(s.items()[0].id, "current");
}

#[test]
fn layout_identity_is_stable_but_callbacks_expire_with_the_query() {
    let mut s = Session::new(History::default(), registry(false)).unwrap();
    let g = start(&mut s, "first");
    s.deliver_from(id("apps.search"), batch(g, &["same"]));
    s.deliver_from(id("slow.search"), batch(g, &["same"]));
    let layout = s.layout_key(0);
    let callback = s.result_key(0);
    assert_ne!(layout, s.layout_key(1));
    let g = start(&mut s, "second");
    s.deliver_from(id("apps.search"), batch(g, &["same"]));
    assert_eq!(s.layout_key(0), layout);
    assert_ne!(s.result_key(0), callback);
    assert!(s.run_key_action(&callback, "open").is_empty());
}

#[test]
fn independent_completion_limits_pinning_and_identity() {
    let mut s = Session::new(History::default(), registry(true)).unwrap();
    let g = start(&mut s, "query");
    s.deliver_from(id("apps.search"), batch(g, &["same", "app2", "app3", "excess"]));
    let app_key = s.result_key(0);
    s.deliver_from(id("apps.search"), Delivery::Done { generation: g });
    assert_eq!(s.items().len(), 3);
    assert!(matches!(s.results(), Results::Searching { replaced: true }));
    s.deliver_from(id("slow.search"), batch(g, &["same", "slow2"]));
    assert_eq!(
        s.items().iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
        ["same", "same", "app2"]
    );
    assert_eq!(s.selected(), 1);
    assert_ne!(s.result_key(0), app_key);
    assert!(s.select_key(&app_key));
    let slow_key = s.result_key(0);
    s.deliver_from(id("slow.search"), Delivery::Failed { generation: g });
    assert!(s.run_key_action(&slow_key, "open").is_empty());
    assert_eq!(s.results(), Results::Done);
    assert_eq!(s.failures().count(), 1);
    assert!(!s.deliver_from(id("slow.search"), batch(g, &["late"])));
    let effects = s.run_key_action(&app_key, "copy-path");
    assert_eq!(effects[0], Effect::Run(Action::Copy("same".into())));
    assert_eq!(s.failures().count(), 0);
}

#[test]
fn expiry_and_stale_callbacks_do_not_close_other_provider() {
    let mut s = Session::new(History::default(), registry(false)).unwrap();
    let g = start(&mut s, "old");
    s.deliver_from(id("slow.search"), batch(g, &["same"]));
    let old = s.result_key(0);
    let next = start(&mut s, "new");
    assert!(!s.select_key(&old));
    assert!(s.run_key_action(&old, "open").is_empty());
    assert!(!s.deliver_from(id("slow.search"), Delivery::Expired { generation: g }));
    assert!(s.deliver_from(id("slow.search"), Delivery::Expired { generation: next }));
    assert!(s.pending(&id("apps.search"), next));
    assert!(!s.deliver_from(id("slow.search"), batch(next, &["expired"])));
    s.deliver_from(id("apps.search"), batch(next, &["app"]));
    s.deliver_from(id("apps.search"), Delivery::Done { generation: next });
    assert_eq!(s.results(), Results::Done);
    assert_eq!(s.items()[0].id, "app");
    assert_eq!(s.failures().count(), 1);
    start(&mut s, "fresh");
    assert_eq!(s.failures().count(), 0);
}

#[test]
fn explicit_routes_do_not_fan_out() {
    let mut s = Session::new(History::default(), registry(false)).unwrap();
    assert_eq!(
        s.set_query("slow term")
            .iter()
            .filter(|e| matches!(e, Effect::Search { .. }))
            .count(),
        1
    );
    assert_eq!(s.provider_ids(), [id("slow.search")]);
}

#[test]
fn fallback_results_wait_for_every_ordinary_provider_and_obey_the_default_limit() {
    let mut session = Session::new(History::default(), fallback_registry()).unwrap();
    let generation = start(&mut session, "unknown query");
    session.deliver_from(
        id("web.fallbacks"),
        batch(generation, &["fallback-1", "fallback-2", "fallback-3"]),
    );
    session.deliver_from(id("web.fallbacks"), Delivery::Done { generation });
    assert!(session.items().is_empty());
    assert_eq!(session.intent(Intent::Open), Some(Vec::new()));
    assert!(session.pending(&id("apps.search"), generation));

    session.deliver_from(id("apps.search"), Delivery::Done { generation });
    assert!(session.items().is_empty());
    assert!(session.pending(&id("web.keywords"), generation));

    session.deliver_from(id("web.keywords"), Delivery::Done { generation });
    assert_eq!(session.results(), Results::Done);
    assert_eq!(visible_ids(&session), ["fallback-1", "fallback-2"]);
    let key = session.result_key(0);
    assert!(matches!(
        session.run_key_action(&key, "open").first(),
        Some(Effect::Run(_))
    ));
}

#[test]
fn fallback_results_are_shown_when_they_arrive_after_empty_searches_finish() {
    let mut session = Session::new(History::default(), fallback_registry()).unwrap();
    let generation = start(&mut session, "unknown query");
    session.deliver_from(id("apps.search"), Delivery::Done { generation });
    session.deliver_from(id("web.keywords"), Delivery::Done { generation });
    assert!(session.items().is_empty());

    session.deliver_from(id("web.fallbacks"), batch(generation, &["fallback"]));
    assert_eq!(visible_ids(&session), ["fallback"]);
    assert!(session.result_key(0).starts_with(&format!("{generation}:")));
}

#[test]
fn fallback_rows_retained_from_an_old_query_cannot_be_opened_or_completed() {
    let mut session = Session::new(History::default(), fallback_registry()).unwrap();
    let generation = start(&mut session, "unknown query");
    session.deliver_from(id("apps.search"), Delivery::Done { generation });
    session.deliver_from(id("web.keywords"), Delivery::Done { generation });
    let mut suggestion = item("fallback");
    suggestion.completion = Some("completed query".into());
    session.deliver_from(
        id("web.fallbacks"),
        Delivery::Batch {
            generation,
            items: vec![suggestion],
        },
    );
    assert_eq!(visible_ids(&session), ["fallback"]);

    let next = start(&mut session, "next query");
    assert_eq!(visible_ids(&session), ["fallback"]);
    assert_eq!(session.intent(Intent::Complete), None);
    assert_eq!(session.intent(Intent::Open), Some(Vec::new()));
    session.deliver_from(id("web.fallbacks"), batch(next, &["stale fallback"]));
    assert!(session.items().is_empty());
}

#[test]
fn ordinary_failures_can_release_fallback_but_expired_partial_rows_suppress_it() {
    let mut session = Session::new(History::default(), fallback_registry()).unwrap();
    let generation = start(&mut session, "unknown query");
    session.deliver_from(id("web.fallbacks"), batch(generation, &["fallback"]));
    session.deliver_from(id("apps.search"), Delivery::Failed { generation });
    assert!(session.items().is_empty());
    session.deliver_from(id("web.keywords"), Delivery::Expired { generation });
    assert_eq!(visible_ids(&session), ["fallback"]);

    let generation = start(&mut session, "another query");
    session.deliver_from(id("web.fallbacks"), batch(generation, &["fallback"]));
    session.deliver_from(id("apps.search"), batch(generation, &["ordinary"]));
    session.deliver_from(id("apps.search"), Delivery::Expired { generation });
    session.deliver_from(id("web.keywords"), Delivery::Done { generation });
    assert_eq!(visible_ids(&session), ["ordinary"]);
}

#[test]
fn a_default_fallback_provider_is_an_ordinary_provider_on_an_explicit_route() {
    let mut commands = fallback_registry();
    commands.add_keyword(&id("web.fallbacks"), "lookup").unwrap();
    let mut session = Session::new(History::default(), commands).unwrap();
    let effects = session.set_query("lookup result");
    let generation = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Search {
                generation, command, ..
            } if command == &id("web.fallbacks") => Some(*generation),
            _ => None,
        })
        .unwrap();
    session.deliver_from(id("web.fallbacks"), batch(generation, &["direct"]));
    assert_eq!(visible_ids(&session), ["direct"]);
}

struct Gated {
    gate: Mutex<mpsc::Receiver<()>>,
    entered: mpsc::Sender<()>,
}
impl Provider for Gated {
    fn search(&self, _: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        self.entered.send(()).unwrap();
        self.gate.lock().unwrap().recv().unwrap();
        emit(vec![item("slow")]);
        Ok(())
    }
}
struct Fast;
impl Provider for Fast {
    fn search(&self, _: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        emit(vec![item("app")]);
        Ok(())
    }
}

#[test]
fn app_is_visible_while_other_worker_is_blocked() {
    let (release, gate) = mpsc::channel();
    let (entered, waiting) = mpsc::channel();
    let (tx, rx) = mpsc::channel();
    let slow_tx = tx.clone();
    let slow = Searcher::spawn(
        "slow",
        Arc::new(Gated {
            gate: Mutex::new(gate),
            entered,
        }),
        move |d| {
            slow_tx.send((id("slow.search"), d)).unwrap();
        },
    );
    let fast = Searcher::spawn("fast", Arc::new(Fast), move |d| {
        tx.send((id("apps.search"), d)).unwrap();
    });
    let mut s = Session::new(History::default(), registry(false)).unwrap();
    let g = start(&mut s, "query");
    slow.search(g, "query".into(), 3);
    waiting.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    fast.search(g, "query".into(), 3);
    for _ in 0..2 {
        let (origin, d) = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(origin, id("apps.search"));
        s.deliver_from(origin, d);
    }
    assert_eq!(s.items()[0].id, "app");
    assert!(s.pending(&id("slow.search"), g));
    release.send(()).unwrap();
    for _ in 0..2 {
        let (origin, d) = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        s.deliver_from(origin, d);
    }
    assert_eq!(s.results(), Results::Done);
}

#[test]
fn completed_order_is_independent_of_provider_arrival_and_refresh_keeps_origin() {
    fn deliver(s: &mut Session, origin: &str, g: u64) {
        s.deliver_from(id(origin), batch(g, &["same", "second"]));
        s.deliver_from(id(origin), Delivery::Done { generation: g });
    }
    let mut a = Session::new(History::default(), registry(true)).unwrap();
    let mut b = Session::new(History::default(), registry(true)).unwrap();
    let g = start(&mut a, "query");
    start(&mut b, "query");
    deliver(&mut a, "apps.search", g);
    deliver(&mut a, "slow.search", g);
    deliver(&mut b, "slow.search", g);
    deliver(&mut b, "apps.search", g);
    assert_eq!(a.items(), b.items());
    for i in 0..a.items().len() {
        assert_eq!(a.result_key(i), b.result_key(i));
    }
    let app = a.result_key(1);
    assert!(a.select_key(&app));
    let effects = a.refresh();
    let next = match effects[0] {
        Effect::Search { generation, .. } => generation,
        _ => panic!(),
    };
    assert!(a.run_key_action(&app, "open").is_empty());
    deliver(&mut a, "slow.search", next);
    deliver(&mut a, "apps.search", next);
    assert_eq!(a.selected(), 1);
}

#[test]
fn duplicate_local_actions_run_only_the_selected_origin_and_keep_history_keys() {
    for index in 0..2 {
        let mut s = Session::new(History::default(), registry(false)).unwrap();
        let g = start(&mut s, "query");
        for (origin, value) in [("apps.search", "app"), ("slow.search", "slow")] {
            let mut row = item("same");
            row.actions[0].action = Action::Copy(value.into());
            s.deliver_from(
                id(origin),
                Delivery::Batch {
                    generation: g,
                    items: vec![row],
                },
            );
        }
        let key = s.result_key(index);
        let expected = if index == 0 { "app" } else { "slow" };
        assert_eq!(
            s.run_key_action(&key, "open")[0],
            Effect::Run(Action::Copy(expected.into()))
        );
        assert!(s.history().boost("same", "query", u64::MAX) > 0);
        assert_eq!(s.history().boost(&key, "query", u64::MAX), 0);
    }
}

#[test]
fn provider_error_is_terminal_and_worker_closes_when_dropped() {
    struct Broken;
    impl Provider for Broken {
        fn search(&self, _: &Request, _: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
            Err(ProviderError)
        }
    }
    let (tx, rx) = mpsc::channel();
    let worker = Searcher::spawn("broken", Arc::new(Broken), move |d| {
        tx.send(d).unwrap();
    });
    worker.search(1, "query".into(), 2);
    assert_eq!(
        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
        Delivery::Failed { generation: 1 }
    );
    drop(worker);
    assert_eq!(
        rx.recv_timeout(std::time::Duration::from_secs(5)),
        Err(mpsc::RecvTimeoutError::Disconnected)
    );
}
