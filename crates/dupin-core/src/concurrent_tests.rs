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
    assert_eq!(s.selected(), 0);
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
fn automatic_keyword_completion_is_independent_of_provider_arrival() {
    for keyword_first in [true, false] {
        let mut session = Session::new(History::default(), fallback_registry()).unwrap();
        let generation = start(&mut session, "g");
        let mut keyword = item("keyword");
        keyword.completion = Some("g ".into());
        let providers = if keyword_first {
            ["web.keywords", "apps.search"]
        } else {
            ["apps.search", "web.keywords"]
        };
        for provider in providers {
            let row = if provider == "web.keywords" {
                keyword.clone()
            } else {
                item("app")
            };
            session.deliver_from(
                id(provider),
                Delivery::Batch {
                    generation,
                    items: vec![row],
                },
            );
        }
        assert_eq!(session.selected_item().unwrap().id, "keyword");
        assert!(
            session
                .intent(Intent::Complete)
                .unwrap()
                .contains(&Effect::ReplaceQuery("g ".into()))
        );
    }
}

#[test]
fn a_deliberate_app_selection_is_not_replaced_by_a_late_keyword() {
    let mut session = Session::new(History::default(), fallback_registry()).unwrap();
    let generation = start(&mut session, "g");
    session.deliver_from(id("apps.search"), batch(generation, &["app"]));
    assert!(session.select_key(&session.result_key(0)));
    let mut keyword = item("keyword");
    keyword.completion = Some("g ".into());
    session.deliver_from(
        id("web.keywords"),
        Delivery::Batch {
            generation,
            items: vec![keyword],
        },
    );
    assert_eq!(session.selected_item().unwrap().id, "app");
    assert_eq!(session.intent(Intent::Complete), None);
}

#[test]
fn a_held_fallback_cannot_displace_an_ordinary_match_after_refresh() {
    let mut session = Session::new(History::default(), fallback_registry()).unwrap();
    let generation = start(&mut session, "unknown");
    session.deliver_from(id("apps.search"), Delivery::Done { generation });
    session.deliver_from(id("web.keywords"), Delivery::Done { generation });
    session.deliver_from(id("web.fallbacks"), batch(generation, &["fallback"]));
    assert!(session.select_key(&session.result_key(0)));
    let generation = session
        .refresh()
        .into_iter()
        .find_map(|effect| match effect {
            Effect::Search { generation, .. } => Some(generation),
            _ => None,
        })
        .unwrap();
    session.deliver_from(id("web.fallbacks"), batch(generation, &["fallback"]));
    session.deliver_from(id("apps.search"), batch(generation, &["app"]));
    assert_eq!(visible_ids(&session), ["app"]);
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
    assert_eq!(visible_ids(&session), ["fallback"]);
    assert_eq!(session.intent(Intent::Complete), None);
    assert_eq!(session.intent(Intent::Open), Some(Vec::new()));
}

#[test]
fn empty_auxiliary_answers_preserve_only_pending_providers_old_rows() {
    let mut session = Session::new(History::default(), fallback_registry()).unwrap();
    let generation = start(&mut session, "first");
    let mut app = item("old app");
    app.completion = Some("old completion".into());
    session.deliver_from(
        id("apps.search"),
        Delivery::Batch {
            generation,
            items: vec![app],
        },
    );
    session.deliver_from(id("web.keywords"), batch(generation, &["old keyword"]));
    let generation = start(&mut session, "second");
    session.deliver_from(id("web.keywords"), Delivery::Done { generation });
    session.deliver_from(id("web.fallbacks"), Delivery::Done { generation });
    assert_eq!(visible_ids(&session), ["old app"]);
    assert_eq!(session.results(), Results::Searching { replaced: false });
    let key = session.result_key(0);
    assert!(session.run_key_action(&key, "open").is_empty());
    assert_eq!(session.intent(Intent::Complete), None);
    session.deliver_from(id("apps.search"), Delivery::Done { generation });
    assert!(session.items().is_empty());
    assert_eq!(session.results(), Results::Done);
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

fn ranked_session(history: History, shown: usize, candidates: usize) -> Session {
    let command = id("rank.search");
    let mut commands = CommandRegistry::new(command.clone(), None);
    commands
        .register(
            CommandSpec::new(command, shown, QueryPresentation::new("Search"))
                .with_candidate_limit(candidates),
        )
        .unwrap();
    Session::new(history, commands).unwrap()
}

fn scored_batch(generation: u64, entries: &[(&str, i32)]) -> Delivery {
    Delivery::Batch {
        generation,
        items: entries
            .iter()
            .map(|(name, score)| {
                let mut row = item(name);
                row.score = Some(*score);
                row
            })
            .collect(),
    }
}

#[test]
fn late_better_candidates_are_ranked_before_the_visible_cut() {
    let mut session = ranked_session(History::default(), 2, 4);
    let generation = start(&mut session, "term");
    session.deliver_from(
        id("rank.search"),
        scored_batch(generation, &[("low", 30), ("medium", 50)]),
    );
    session.deliver_from(
        id("rank.search"),
        scored_batch(generation, &[("best", 100), ("good", 90)]),
    );
    assert_eq!(visible_ids(&session), ["best", "good"]);
    session.deliver_from(
        id("rank.search"),
        scored_batch(generation, &[("over-budget", 1000)]),
    );
    assert_eq!(visible_ids(&session), ["best", "good"]);
}

#[test]
fn an_explicit_selection_survives_falling_below_the_visible_cut() {
    let mut session = ranked_session(History::default(), 2, 4);
    let generation = start(&mut session, "term");
    session.deliver_from(
        id("rank.search"),
        scored_batch(generation, &[("low", 30), ("medium", 50)]),
    );
    let chosen = session.result_key(1);
    assert!(session.select_key(&chosen));
    session.deliver_from(
        id("rank.search"),
        scored_batch(generation, &[("best", 100), ("good", 90)]),
    );
    assert_eq!(visible_ids(&session), ["best", "low"]);
    assert_eq!(session.selected_item().unwrap().id, "low");
    assert_eq!(session.result_key(session.selected()), chosen);
    assert!(!session.run_key_action(&chosen, "open").is_empty());
}

#[test]
fn an_open_menu_keeps_its_result_visible_during_ranking() {
    let mut session = ranked_session(History::default(), 1, 2);
    let generation = start(&mut session, "term");
    session.deliver_from(id("rank.search"), scored_batch(generation, &[("first", 30)]));
    assert!(matches!(
        &session.intent(Intent::Actions).unwrap()[..],
        [Effect::ShowActions { .. }]
    ));
    session.deliver_from(id("rank.search"), scored_batch(generation, &[("better", 100)]));
    assert_eq!(visible_ids(&session), ["first"]);
}

#[test]
fn an_automatic_first_row_does_not_pin_a_one_result_search() {
    let mut session = ranked_session(History::default(), 1, 2);
    let generation = start(&mut session, "term");
    session.deliver_from(id("rank.search"), scored_batch(generation, &[("first", 30)]));
    session.deliver_from(id("rank.search"), scored_batch(generation, &[("better", 100)]));
    assert_eq!(visible_ids(&session), ["better"]);
}

#[test]
fn query_history_is_applied_before_truncating_candidates() {
    let mut history = History::default();
    history.record("familiar", "term", 0);
    let mut session = ranked_session(history, 1, 2);
    let generation = start(&mut session, "term");
    session.deliver_from(id("rank.search"), scored_batch(generation, &[("new", 100)]));
    session.deliver_from(id("rank.search"), scored_batch(generation, &[("familiar", 90)]));
    assert_eq!(visible_ids(&session), ["familiar"]);
    assert_eq!(session.items()[0].score, Some(90));
}

#[test]
fn duplicate_candidates_do_not_fill_the_bounded_pool() {
    let mut session = ranked_session(History::default(), 2, 3);
    let generation = start(&mut session, "term");
    session.deliver_from(
        id("rank.search"),
        scored_batch(generation, &[("same", 30), ("same", 30)]),
    );
    session.deliver_from(
        id("rank.search"),
        scored_batch(generation, &[("best", 100), ("next", 90)]),
    );
    assert_eq!(visible_ids(&session), ["best", "next"]);
}

#[test]
fn the_worker_candidate_budget_is_cumulative_across_batches() {
    struct Verbose;
    impl Provider for Verbose {
        fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
            assert_eq!(request.limit, 3);
            for names in [["a", "b"], ["c", "d"], ["e", "f"]] {
                emit(names.into_iter().map(item).collect());
            }
            Ok(())
        }
    }
    let (sender, receiver) = mpsc::channel();
    let worker = Searcher::spawn("candidate-budget", Arc::new(Verbose), move |delivery| {
        sender.send(delivery).unwrap()
    });
    worker.search(1, "term".into(), 3);
    let mut total = 0;
    loop {
        match receiver.recv_timeout(std::time::Duration::from_secs(5)).unwrap() {
            Delivery::Batch { items, .. } => total += items.len(),
            Delivery::Done { .. } => break,
            other => panic!("unexpected delivery: {other:?}"),
        }
    }
    assert_eq!(total, 3);
}

#[test]
fn a_refresh_keeps_the_chosen_identity_in_its_first_ranked_batch() {
    let mut session = ranked_session(History::default(), 1, 3);
    let generation = start(&mut session, "term");
    session.deliver_from(id("rank.search"), scored_batch(generation, &[("chosen", 30)]));
    let chosen = session.result_key(0);
    assert!(session.select_key(&chosen));
    let next = session
        .refresh()
        .iter()
        .find_map(|effect| match effect {
            Effect::Search { generation, .. } => Some(*generation),
            _ => None,
        })
        .unwrap();
    session.deliver_from(
        id("rank.search"),
        scored_batch(next, &[("best", 100), ("chosen", 30)]),
    );
    assert_eq!(visible_ids(&session), ["chosen"]);
    assert_eq!(session.selected_item().unwrap().id, "chosen");
    assert_ne!(session.result_key(0), chosen);
}

#[test]
fn holding_a_selection_does_not_carry_into_a_different_query() {
    let mut session = ranked_session(History::default(), 1, 3);
    let generation = start(&mut session, "old");
    session.deliver_from(id("rank.search"), scored_batch(generation, &[("chosen", 30)]));
    let chosen = session.result_key(0);
    assert!(session.select_key(&chosen));
    let next = start(&mut session, "new");
    session.deliver_from(id("rank.search"), scored_batch(next, &[("first", 30)]));
    session.deliver_from(id("rank.search"), scored_batch(next, &[("better", 100)]));
    assert_eq!(visible_ids(&session), ["better"]);
}

#[test]
fn a_sole_pinned_slot_is_not_replaced_to_keep_another_providers_selection() {
    let command = id("rank.search");
    let pin = id("pin.search");
    let mut commands = CommandRegistry::new(command.clone(), None);
    commands
        .register(
            CommandSpec::new(command.clone(), 1, QueryPresentation::new("Search")).with_candidate_limit(2),
        )
        .unwrap();
    commands
        .register(CommandSpec::new(pin.clone(), 1, QueryPresentation::new("Pin")))
        .unwrap();
    commands
        .set_default_providers(vec![command.clone(), pin.clone()], Some(pin.clone()))
        .unwrap();
    let mut session = Session::new(History::default(), commands).unwrap();
    let generation = start(&mut session, "term");
    session.deliver_from(command, scored_batch(generation, &[("chosen", 30)]));
    assert!(session.select_key(&session.result_key(0)));
    session.deliver_from(pin, scored_batch(generation, &[("pinned", 100)]));
    assert_eq!(visible_ids(&session), ["pinned"]);
}

#[test]
fn retaining_a_pinned_selection_never_uses_a_second_slot() {
    let main = id("rank.search");
    let pin = id("pin.search");
    let mut commands = CommandRegistry::new(main.clone(), None);
    commands
        .register(CommandSpec::new(
            main.clone(),
            2,
            QueryPresentation::new("Search"),
        ))
        .unwrap();
    commands
        .register(CommandSpec::new(pin.clone(), 3, QueryPresentation::new("Pin")).with_candidate_limit(4))
        .unwrap();
    commands
        .set_default_providers(vec![main.clone(), pin.clone()], Some(pin.clone()))
        .unwrap();
    let mut session = Session::new(History::default(), commands).unwrap();
    let generation = start(&mut session, "term");
    session.deliver_from(pin.clone(), scored_batch(generation, &[("chosen-pin", 30)]));
    assert!(session.select_key(&session.result_key(0)));
    session.deliver_from(main, scored_batch(generation, &[("main-row", 100)]));
    session.deliver_from(pin, scored_batch(generation, &[("better-pin", 100)]));
    assert_eq!(visible_ids(&session), ["chosen-pin", "main-row"]);
}

#[test]
fn a_refresh_does_not_pin_its_automatic_first_row_below_the_cut() {
    let mut session = ranked_session(History::default(), 1, 3);
    let generation = start(&mut session, "term");
    session.deliver_from(id("rank.search"), scored_batch(generation, &[("first", 30)]));
    let next = session
        .refresh()
        .iter()
        .find_map(|effect| match effect {
            Effect::Search { generation, .. } => Some(*generation),
            _ => None,
        })
        .unwrap();
    session.deliver_from(
        id("rank.search"),
        scored_batch(next, &[("first", 30), ("better", 100)]),
    );
    assert_eq!(visible_ids(&session), ["better"]);
}

#[test]
fn a_refresh_does_not_restore_an_automatic_selection_still_inside_the_cut() {
    let mut session = ranked_session(History::default(), 2, 3);
    let generation = start(&mut session, "term");
    session.deliver_from(id("rank.search"), scored_batch(generation, &[("first", 30)]));
    let generation = session
        .refresh()
        .into_iter()
        .find_map(|effect| match effect {
            Effect::Search { generation, .. } => Some(generation),
            _ => None,
        })
        .unwrap();
    session.deliver_from(
        id("rank.search"),
        scored_batch(generation, &[("first", 30), ("better", 100)]),
    );
    assert_eq!(visible_ids(&session), ["better", "first"]);
    assert_eq!(session.selected_item().unwrap().id, "better");
}

#[test]
fn a_new_menu_target_replaces_the_identity_held_before_refresh() {
    let mut session = ranked_session(History::default(), 1, 3);
    let generation = start(&mut session, "term");
    session.deliver_from(id("rank.search"), scored_batch(generation, &[("old", 30)]));
    assert!(session.select_key(&session.result_key(0)));
    let generation = session
        .refresh()
        .into_iter()
        .find_map(|effect| match effect {
            Effect::Search { generation, .. } => Some(generation),
            _ => None,
        })
        .unwrap();
    session.deliver_from(id("rank.search"), scored_batch(generation, &[("new", 100)]));
    let target = session.result_key(0);
    assert!(matches!(
        &session.intent(Intent::Actions).unwrap()[..],
        [Effect::ShowActions { id, .. }] if *id == target
    ));
    session.deliver_from(id("rank.search"), scored_batch(generation, &[("old", 200)]));
    assert_eq!(visible_ids(&session), ["new"]);
    assert!(!session.run_key_action(&target, "open").is_empty());
}
