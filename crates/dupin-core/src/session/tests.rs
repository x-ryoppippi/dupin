use std::path::PathBuf;

use super::*;
use crate::{Action, CommandId, CommandRegistry, CommandSpec, QueryEligibility, QueryPresentation, View};

fn trash_row() -> Item {
    let mut row = item("file");
    row.actions.push(ItemAction::new(
        "choose",
        "Open with…",
        Action::ChooseApplication("/file".into()),
    ));
    row.actions.push(
        ItemAction::new("trash", "Trash", Action::Trash("/file".into()))
            .confirm("Trash?", "Trash")
            .consumes(),
    );
    row.primary = Some("trash".into());
    row.alternate = Some("trash".into());
    row
}

fn file_action_session() -> (Session, String, u64) {
    let mut s = session();
    let generation = searched(&s.set_query("file")).1;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![trash_row()],
    });
    let key = s.result_key(0);
    (s, key, generation)
}

#[test]
fn confirmed_consumption_cannot_run_from_normal_activation() {
    let (mut s, key, generation) = file_action_session();
    let history = s.history().clone();
    for intent in [Intent::Open, Intent::OpenAlternate] {
        assert_eq!(
            s.intent(intent).unwrap(),
            [Effect::ShowActions {
                id: key.clone(),
                generation,
                confirmation: Some("trash".into()),
            }]
        );
    }
    assert!(s.run_key_action(&key, "trash").is_empty());
    assert!(s.run_key_action(&key, "choose").is_empty());
    assert!(s.run_confirmed_key_action(&key, "choose").is_empty());
    assert_eq!(
        s.run_confirmed_key_action(&key, "trash"),
        vec![Effect::RunConsuming {
            id: key.clone(),
            action: Action::Trash("/file".into())
        }]
    );
    assert_eq!(&*s.history(), &history);
    s.set_query("other");
    assert!(s.run_confirmed_key_action(&key, "trash").is_empty());
}

#[test]
fn application_choices_belong_to_the_current_parent_resource() {
    let (mut s, key, _) = file_action_session();
    let choice = |path: &str| Action::OpenWith {
        path: path.into(),
        application: "/Viewer.app".into(),
    };
    assert!(s.run_key_choice(&key, "choose", choice("/wrong")).is_empty());
    assert!(s.run_key_choice(&key, "trash", choice("/file")).is_empty());
    assert!(
        s.run_key_choice(&key, "choose", Action::Trash("/file".into()))
            .is_empty()
    );
    let effects = s.run_key_choice(&key, "choose", choice("/file"));
    assert!(effects.contains(&Effect::Run(choice("/file"))));
    assert!(effects.contains(&Effect::SaveHistory));
    assert!(effects.contains(&Effect::Hide));
    assert!(s.run_key_choice(&key, "choose", choice("/file")).is_empty());
}

#[test]
fn consumed_rows_cannot_return_in_late_batches_or_remove_new_generations() {
    let (mut s, key, generation) = file_action_session();
    assert!(s.forget(&key));
    assert!(s.items().is_empty());
    s.deliver(Delivery::Batch {
        generation,
        items: vec![item("file")],
    });
    assert!(s.items().is_empty());
    let generation = searched(&s.set_query("new file")).1;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![item("file")],
    });
    assert!(!s.forget(&key));
    assert_eq!(s.items().len(), 1);
}

#[test]
fn consumed_candidates_cannot_return_after_becoming_hidden() {
    let mut commands = CommandRegistry::new(app_command(), None);
    commands
        .register(
            CommandSpec::new(app_command(), 10, QueryPresentation::new("Search")).with_candidate_limit(20),
        )
        .unwrap();
    let mut s = Session::new(History::default(), commands).unwrap();
    let generation = searched(&s.set_query("file")).1;
    let mut row = trash_row();
    row.score = Some(0);
    s.deliver(Delivery::Batch {
        generation,
        items: vec![row.clone()],
    });
    let key = s.result_key(0);
    s.deliver(Delivery::Batch {
        generation,
        items: (0..10)
            .map(|i| {
                let mut other = item(&format!("other-{i}"));
                other.score = Some(100 + i);
                other
            })
            .collect(),
    });
    assert!(!s.items().iter().any(|item| item.id == row.id));
    let selected = s.selected_item().unwrap().id.clone();
    assert!(s.forget(&key));
    assert_eq!(s.selected_item().unwrap().id, selected);
    assert!(
        s.search
            .providers
            .values()
            .all(|state| state.items.iter().all(|item| item.id != row.id))
    );
    row.score = Some(i32::MAX);
    s.deliver(Delivery::Batch {
        generation,
        items: vec![row.clone()],
    });
    s.deliver(Delivery::Done { generation });
    assert!(!s.items().iter().any(|item| item.id == row.id));
}

#[test]
fn consuming_an_earlier_row_keeps_the_same_item_selected() {
    let (mut s, key, generation) = file_action_session();
    s.deliver(Delivery::Batch {
        generation,
        items: vec![item("next"), item("last")],
    });
    s.intent(Intent::Down);
    let selected = s.selected_item().unwrap().id.clone();
    assert!(s.forget(&key));
    assert_eq!(s.selected_item().unwrap().id, selected);
}

fn command(value: &str) -> CommandId {
    CommandId::new(value).unwrap()
}

fn app_command() -> CommandId {
    command("sample.search")
}

fn files_command() -> CommandId {
    command("sample.files")
}

fn contents_command() -> CommandId {
    command("sample.contents")
}

fn dictionary_command() -> CommandId {
    command("sample.lookup")
}

fn browse_command() -> CommandId {
    command("sample.browse")
}

fn command_registry(dictionary_keywords: Vec<String>) -> CommandRegistry {
    let app = app_command();
    let files = files_command();
    let contents = contents_command();
    let dictionary = dictionary_command();
    let mut registry = CommandRegistry::new(app.clone(), Some(files.clone()));
    for (id, label) in [
        (app, "Search"),
        (files, "Files"),
        (contents.clone(), "Contents"),
        (dictionary.clone(), "Lookup"),
    ] {
        registry
            .register(CommandSpec::new(id, 10, QueryPresentation::new(label)))
            .unwrap();
    }
    registry
        .register(
            CommandSpec::new(browse_command(), 10, QueryPresentation::new("Folder"))
                .with_query_eligibility(crate::QueryEligibility::Any),
        )
        .unwrap();
    registry.set_browser(browse_command()).unwrap();
    registry.add_keyword(&contents, "in").unwrap();
    for keyword in dictionary_keywords {
        registry.add_keyword(&dictionary, keyword).unwrap();
    }
    registry
}

fn session() -> Session {
    session_with_keywords(History::default(), vec!["define".into(), "d".into()])
}

fn session_with_keywords(history: History, keywords: Vec<String>) -> Session {
    Session::new(history, command_registry(keywords)).unwrap()
}

fn session_with_clock(history: History, clock: fn() -> u64) -> Session {
    Session::with_clock(
        history,
        command_registry(vec!["define".into(), "d".into()]),
        clock,
    )
    .unwrap()
}

fn empty_provider_registry(include_empty_provider: bool) -> (CommandRegistry, CommandId, CommandId) {
    let default = app_command();
    let empty = command("sample.empty");
    let lookup = dictionary_command();
    let mut registry = CommandRegistry::new(default.clone(), None);
    registry
        .register(CommandSpec::new(
            default.clone(),
            10,
            QueryPresentation::new("Search"),
        ))
        .unwrap();
    registry
        .register(
            CommandSpec::new(empty.clone(), 10, QueryPresentation::new("Empty"))
                .with_query_eligibility(QueryEligibility::EmptyOnly),
        )
        .unwrap();
    registry.add_keyword(&empty, "blank").unwrap();
    registry
        .register(CommandSpec::new(
            lookup.clone(),
            10,
            QueryPresentation::new("Lookup"),
        ))
        .unwrap();
    registry.add_keyword(&lookup, "lookup").unwrap();
    registry
        .set_default_providers(
            if include_empty_provider {
                vec![default.clone(), empty.clone()]
            } else {
                vec![default.clone()]
            },
            include_empty_provider.then(|| empty.clone()),
        )
        .unwrap();
    (registry, default, empty)
}

fn session_with_empty_provider() -> (Session, CommandId, CommandId) {
    let (registry, default, empty) = empty_provider_registry(true);
    (
        Session::new(History::default(), registry).unwrap(),
        default,
        empty,
    )
}

fn item(name: &str) -> Item {
    Item::path(PathBuf::from(format!("/x/{name}")), name.to_owned(), vec![])
}

fn completion_item(name: &str, completion: &str) -> Item {
    let mut item = item(name);
    item.completion = Some(completion.to_owned());
    item
}

fn searched(effects: &[Effect]) -> (CommandId, u64, &str) {
    match effects {
        [
            Effect::Search {
                command,
                generation,
                query,
                ..
            },
        ] => (command.clone(), *generation, query),
        other => panic!("expected one search, got {other:?}"),
    }
}

#[test]
fn providers_and_recall_keep_escaped_trailing_whitespace() {
    let raw = "in:~/Work\\ ";
    for delete_before_dismiss in [false, true] {
        let mut s = session();
        s.intent(Intent::Space);
        let effects = s.set_query(raw);
        let (_, generation, query) = searched(&effects);
        assert_eq!(query, raw);
        assert_eq!(
            crate::file_query::FileQuery::parse(query).unwrap().folder,
            crate::file_query::FileQuery::parse("in:\"~/Work \"")
                .unwrap()
                .folder
        );
        s.deliver(Delivery::Done { generation });
        if delete_before_dismiss {
            s.set_query("in:~/Work\\");
        }
        s.intent(Intent::Dismiss);
        s.intent(Intent::Space);
        let effects = s.intent(Intent::Up).unwrap();
        assert!(effects.contains(&Effect::ReplaceQuery(raw.into())));
        assert_eq!(s.query(), raw);
    }
}

#[test]
fn starts_in_apps_mode_with_nothing_shown() {
    let s = session();
    assert_eq!(s.command_id(), &app_command());
    assert_eq!(s.results(), Results::Idle);
    assert!(s.items().is_empty());
}

#[test]
fn empty_queries_search_only_eligible_providers_and_keep_pinning() {
    let (mut s, default, empty) = session_with_empty_provider();
    let effects = s.refresh();
    let (provider, generation, query) = searched(&effects);
    assert_eq!(provider, empty);
    assert_eq!(query, "");
    assert!(!s.pending(&default, generation));
    assert!(s.pending(&provider, generation));

    assert!(s.deliver_from(
        provider.clone(),
        Delivery::Batch {
            generation,
            items: vec![item("empty-a"), item("empty-b")],
        }
    ));
    assert_eq!(
        s.items().len(),
        1,
        "the eligible pinned provider keeps its pin limit"
    );
}

#[test]
fn explicit_blank_commands_resume_default_providers_only_after_backspace() {
    let (mut s, default, empty) = session_with_empty_provider();
    let routed = s.set_query("lookup term");
    assert!(matches!(
        routed.iter().find(|effect| matches!(effect, Effect::Search { .. })),
        Some(Effect::Search { command, query, .. }) if command == &dictionary_command() && query == "term"
    ));

    let effects = s.set_query("");
    assert_eq!(effects, [Effect::CancelSearch]);
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Search { command, .. } if command == &empty))
    );
    assert_eq!(s.results(), Results::Idle);

    let effects = s.intent(Intent::Backspace).unwrap();
    let (provider, generation, query) = searched(&effects);
    assert_eq!(provider, empty);
    assert_eq!(query, "");
    assert_eq!(s.command_id(), &default);
    assert!(!s.search.explicit);
    assert!(s.pending(&empty, generation));
    assert!(!s.pending(&default, generation));
}

#[test]
fn an_explicit_empty_only_command_searches_its_own_provider() {
    let (mut s, default, empty) = session_with_empty_provider();
    s.set_query("blank");
    let effects = s.intent(Intent::Space).unwrap();
    assert_eq!(s.command_id(), &empty);
    assert!(s.search.explicit);
    let generation = match effects.as_slice() {
        [
            Effect::ClearQuery,
            Effect::Search {
                command,
                generation,
                query,
                ..
            },
        ] if command == &empty && query.is_empty() => *generation,
        other => panic!("expected one empty-provider search, got {other:?}"),
    };
    assert!(!s.pending(&default, generation));
    assert!(s.pending(&empty, generation));
}

#[test]
fn typing_and_clearing_replace_generations_for_the_eligible_provider_set() {
    let (mut s, default, empty) = session_with_empty_provider();
    let empty_generation = searched(&s.refresh()).1;
    assert!(s.deliver_from(
        empty.clone(),
        Delivery::Batch {
            generation: empty_generation,
            items: vec![item("suggestion")],
        }
    ));
    assert_eq!(s.items().len(), 1);
    let typed = s.set_query("term");
    let (provider, typed_generation, query) = searched(&typed);
    assert_eq!(provider, default);
    assert_eq!(query, "term");
    assert!(s.items().is_empty());
    assert!(!s.deliver_from(
        empty.clone(),
        Delivery::Batch {
            generation: empty_generation,
            items: vec![item("stale-empty")],
        }
    ));

    let cleared = s.set_query("");
    let (provider, cleared_generation, query) = searched(&cleared);
    assert_eq!(provider, empty);
    assert_eq!(query, "");
    assert!(!s.deliver_from(
        default,
        Delivery::Batch {
            generation: typed_generation,
            items: vec![item("stale-typed")],
        }
    ));
    assert!(s.pending(&empty, cleared_generation));
    assert!(s.items().is_empty());
}

#[test]
fn up_can_return_to_the_first_suggestion_after_moving_down() {
    let (mut s, _, empty) = session_with_empty_provider();
    let (mut registry, _, _) = empty_provider_registry(true);
    registry
        .set_default_providers(vec![app_command(), empty.clone()], None)
        .unwrap();
    s.set_commands(registry).unwrap();
    let generation = searched(&s.refresh()).1;
    s.deliver_from(
        empty,
        Delivery::Batch {
            generation,
            items: vec![item("first"), item("second")],
        },
    );

    s.intent(Intent::Down).unwrap();
    assert_eq!(s.selected(), 1);
    s.intent(Intent::Up).unwrap();
    assert_eq!(s.selected(), 0);
}

#[test]
fn opening_a_result_starts_the_empty_search_with_the_new_pick() {
    let (mut s, default, recent) = session_with_empty_provider();
    let history = s.recent_history();
    let generation = searched(&s.set_query("example")).1;
    s.deliver_from(
        default,
        Delivery::Batch {
            generation,
            items: vec![item("example")],
        },
    );

    let effects = s.intent(Intent::Open).unwrap();
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::Run(_), Effect::SaveHistory, Effect::Hide, Effect::Search { command, query, .. }]
                if *command == recent && query.is_empty()
        ),
        "{effects:?}"
    );
    assert_eq!(history.ranked_ids(), ["/x/example"]);
}

#[test]
fn removing_the_only_empty_query_provider_cancels_and_rejects_its_delivery() {
    let (mut s, _, empty) = session_with_empty_provider();
    let generation = searched(&s.refresh()).1;
    let (replacement, _, _) = empty_provider_registry(false);

    assert_eq!(s.set_commands(replacement).unwrap(), [Effect::CancelSearch]);
    assert_eq!(s.results(), Results::Idle);
    assert!(!s.deliver_from(
        empty,
        Delivery::Batch {
            generation,
            items: vec![item("stale")],
        }
    ));
    assert!(s.items().is_empty());
}

#[test]
fn actions_run_the_named_result_even_after_selection_moves() {
    let mut s = session();
    let generation = searched(&s.set_query("note")).1;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![item("notes.md"), item("other.md")],
    });
    let key = s.result_key(0);
    assert_eq!(
        s.intent(Intent::Actions).unwrap(),
        vec![Effect::ShowActions {
            id: s.result_key(0),
            generation,
            confirmation: None,
        }]
    );
    s.intent(Intent::Down);
    let effects = s.run_key_action(&key, "copy-path");
    assert_eq!(effects[0], Effect::Run(Action::Copy("/x/notes.md".into())));
    assert!(effects.contains(&Effect::Hide));
    assert!(s.items().is_empty());
}

#[test]
fn followup_actions_keep_the_parent_target_and_use_their_own_disposition() {
    for keep_open in [true, false] {
        let mut s = session();
        let generation = searched(&s.set_query("notes")).1;
        let mut row = item("notes.md");
        row.actions
            .push(ItemAction::new("inspect", "Inspect", Action::Preview("/x/notes.md".into())).keep_open());
        s.deliver(Delivery::Batch {
            generation,
            items: vec![row],
        });
        let key = s.result_key(0);
        assert_eq!(
            s.run_key_action(&key, "inspect"),
            [Effect::Run(Action::Preview("/x/notes.md".into()))]
        );
        let target = s.followup_target("inspect").unwrap();
        let generation = searched(&s.refresh()).1;
        s.deliver(Delivery::Batch {
            generation,
            items: vec![item("another.md")],
        });
        let mut followup = ItemAction::new("chosen", "Chosen action", Action::Copy("value".into()));
        if keep_open {
            followup = followup.keep_open();
        }
        let effects = s.run_followup(target, followup);
        assert_eq!(effects[0], Effect::Run(Action::Copy("value".into())));
        assert_eq!(effects.contains(&Effect::SaveHistory), !keep_open);
        assert_eq!(effects.contains(&Effect::Hide), !keep_open);
        if keep_open {
            assert_eq!(&*s.history(), &History::default());
            assert_eq!(s.items()[0].id, item("another.md").id);
            assert_eq!(s.query(), "notes");
        } else {
            assert_eq!(s.history().boost(&item("another.md").id, "notes", 0), 0);
            assert!(s.history().boost(&item("notes.md").id, "notes", 0) > 0);
        }
    }
}

#[test]
fn followup_targets_require_an_actionable_parent_and_the_original_query() {
    for reset in [false, true] {
        let mut s = session();
        let generation = searched(&s.set_query("notes")).1;
        s.deliver(Delivery::Batch {
            generation,
            items: vec![item("notes.md")],
        });
        assert!(s.followup_target("missing").is_none());
        let target = s.followup_target("open").unwrap();
        if reset {
            s.reset();
        } else {
            s.set_query("other");
        }
        assert!(s.followup_target("open").is_none());
        let followup = ItemAction::new("chosen", "Chosen action", Action::Copy("value".into()));
        assert!(s.run_followup(target, followup).is_empty());
        assert_eq!(&*s.history(), &History::default());
    }
}

#[test]
fn running_an_inline_script_records_the_pick_and_query_before_dismissal() {
    use crate::script::{OutputMode, ScriptRun};
    let mut s = session();
    let generation = searched(&s.set_query("fixture")).1;
    let mut row = item("fixture");
    row.primary = Some("run".into());
    row.actions.push(
        ItemAction::new(
            "run",
            "Run script",
            Action::RunScript(ScriptRun {
                path: "/fixture".into(),
                title: "Fixture".into(),
                args: vec![],
                mode: OutputMode::Inline,
            }),
        )
        .keep_open(),
    );
    let id = row.id.clone();
    s.deliver(Delivery::Batch {
        generation,
        items: vec![row],
    });
    let effects = s.intent(Intent::Open).unwrap();
    assert!(matches!(
        &effects[..],
        [Effect::Run(Action::RunScript(_)), Effect::SaveHistory]
    ));
    assert_eq!(s.query(), "fixture");
    assert!(s.history().boost(&id, "fixture", 0) > 0);
    assert_eq!(s.queries, [(app_command(), "fixture".into())]);
}

#[test]
fn keep_open_actions_preserve_the_search_and_do_not_record_a_pick() {
    for entry in 0..3 {
        let mut s = session();
        let generation = searched(&s.set_query("note")).1;
        let mut result = item("notes.md");
        result
            .actions
            .push(ItemAction::new("inspect", "Inspect", Action::Preview("/x/notes.md".into())).keep_open());
        result.primary = Some("inspect".into());
        result.alternate = Some("inspect".into());
        s.deliver(Delivery::Batch {
            generation,
            items: vec![item("other.md"), result],
        });
        s.intent(Intent::Down);
        let key = s.result_key(1);
        let before = s.items().to_vec();
        let effects = match entry {
            0 => s.run_key_action(&key, "inspect"),
            1 => s.intent(Intent::Open).unwrap(),
            _ => s.intent(Intent::OpenAlternate).unwrap(),
        };
        assert_eq!(effects, vec![Effect::Run(Action::Preview("/x/notes.md".into()))]);
        assert_eq!(s.query(), "note");
        assert_eq!(s.items(), before);
        assert_eq!(s.result_key(1), key);
        assert_eq!(s.selected_item(), before.get(1));
        assert_eq!(&*s.history(), &History::default());
        assert!(s.queries.is_empty());
    }
}

#[test]
fn keep_open_actions_cannot_preview_stale_or_retained_results() {
    let mut s = session();
    let generation = searched(&s.set_query("old")).1;
    let mut result = item("notes.md");
    result
        .actions
        .push(ItemAction::new("inspect", "Inspect", Action::Preview("/x/notes.md".into())).keep_open());
    s.deliver(Delivery::Batch {
        generation,
        items: vec![result.clone()],
    });
    let old_key = s.result_key(0);
    let generation = searched(&s.set_query("new")).1;
    assert!(s.run_key_action(&old_key, "inspect").is_empty());
    let retained_key = s.result_key(0);
    assert!(s.run_key_action(&retained_key, "inspect").is_empty());
    s.deliver(Delivery::Batch {
        generation,
        items: vec![result],
    });
    assert!(s.run_key_action(&old_key, "inspect").is_empty());
    let fresh_key = s.result_key(0);
    assert_eq!(
        s.run_key_action(&fresh_key, "inspect"),
        vec![Effect::Run(Action::Preview("/x/notes.md".into()))]
    );
    assert_eq!(s.query(), "new");
    assert_eq!(&*s.history(), &History::default());
}

#[test]
fn menu_callbacks_cannot_run_after_edit_refresh_or_dismissal() {
    for boundary in 0..3 {
        let mut s = session();
        let generation = searched(&s.set_query("note")).1;
        s.deliver(Delivery::Batch {
            generation,
            items: vec![item("notes.md")],
        });
        let key = s.result_key(0);
        match boundary {
            0 => {
                s.set_query("other");
            }
            1 => {
                s.refresh();
            }
            _ => {
                s.reset();
            }
        }
        assert!(s.run_key_action(&key, "open").is_empty());
    }
}

#[test]
fn unknown_actions_and_missing_results_do_nothing() {
    let mut s = session();
    assert!(s.intent(Intent::Actions).unwrap().is_empty());
    let generation = searched(&s.set_query("note")).1;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![item("notes.md")],
    });
    let key = s.result_key(0);
    assert!(s.run_key_action(&key, "unknown").is_empty());
    let missing_key = key.replace("/x/notes.md", "/x/missing.md");
    assert!(s.run_key_action(&missing_key, "open").is_empty());
    assert_eq!(s.query(), "note");
}

#[test]
fn retained_rows_are_not_actionable_until_the_new_search_replaces_them() {
    for intent in [Intent::Actions, Intent::Open, Intent::OpenAlternate] {
        let mut s = session();
        let old_generation = searched(&s.set_query("old")).1;
        s.deliver(Delivery::Batch {
            generation: old_generation,
            items: vec![item("old.md")],
        });
        let old_key = s.result_key(0);
        let generation = searched(&s.set_query("new")).1;
        assert_eq!(s.items()[0].id, "/x/old.md");
        assert!(s.intent(intent).unwrap().is_empty());
        assert!(s.run_key_action(&old_key, "open").is_empty());
        let retained_key = s.result_key(0);
        assert!(s.run_key_action(&retained_key, "open").is_empty());
        assert_eq!(&*s.history(), &History::default());
        s.deliver(Delivery::Batch {
            generation,
            items: vec![item("new.md")],
        });
        assert!(!s.intent(intent).unwrap().is_empty());
    }
}

#[test]
fn opening_a_dictionary_result_does_not_change_ranking_history() {
    let mut history = History::default();
    history.record("/Applications/Safari.app", "saf", 1_800_000_000);
    let mut s = session_with_keywords(history.clone(), vec!["define".into(), "d".into()]);
    s.set_query("d");
    s.intent(Intent::Space);
    let generation = searched(&s.set_query("serendipity")).1;
    let mut result = item("serendipity");
    result.record_usage = false;
    result.view = View::Detail {
        title: "serendipity".into(),
        body: "Definition".into(),
    };
    let action = Action::OpenUrl("dict://serendipity".into());
    result.primary = None;
    result.actions = vec![crate::ItemAction::new(
        "dictionary",
        "Open in Dictionary",
        action.clone(),
    )];
    s.deliver(Delivery::Batch {
        generation,
        items: vec![result],
    });
    assert!(matches!(
        s.intent(Intent::Open).unwrap().as_slice(),
        [Effect::ShowActions { .. }]
    ));
    let key = s.result_key(0);
    let effects = s.run_key_action(&key, "dictionary");
    assert!(effects.contains(&Effect::Run(action)));
    assert!(!effects.contains(&Effect::SaveHistory));
    assert_eq!(&*s.history(), &history);
}

#[test]
fn typing_searches_apps() {
    let mut s = session();
    let effects = s.set_query("saf");
    assert_eq!(searched(&effects).0, app_command());
    assert_eq!(searched(&effects).2, "saf");
}

#[test]
fn a_registered_extension_command_owns_its_trigger_and_search_identity() {
    let launcher = command("shell.launch");
    let converter = command("measure.convert");
    let mut registry = CommandRegistry::new(launcher.clone(), None);
    registry
        .register(CommandSpec::new(
            launcher,
            50,
            QueryPresentation::new("Type to launch"),
        ))
        .unwrap();
    registry
        .register(CommandSpec::new(
            converter.clone(),
            1,
            QueryPresentation::new("Convert a value"),
        ))
        .unwrap();
    registry.add_keyword(&converter, "convert").unwrap();
    let mut s = Session::new(History::default(), registry).unwrap();

    s.set_query("convert");
    assert!(s.intent(Intent::Space).unwrap().contains(&Effect::ClearQuery));
    assert_eq!(s.command_id(), &converter);
    assert_eq!(
        searched(&s.set_query("12 km to mi")).0,
        converter,
        "the session routes by the registered ID rather than a feature enum"
    );
}

#[test]
fn a_literal_query_preserves_a_matching_keyword() {
    let mut s = session();
    let effects = s.set_literal_query("define a term");
    assert_eq!(searched(&effects).0, app_command());
    assert_eq!(s.query(), "define a term");
}

#[test]
fn pasted_keywords_use_the_same_modes_and_aliases_as_typed_keywords() {
    for (keyword, mode) in [
        ("in", contents_command()),
        ("define", dictionary_command()),
        ("d", dictionary_command()),
        ("word", dictionary_command()),
        ("look up", dictionary_command()),
    ] {
        let mut s = session_with_keywords(
            History::default(),
            vec!["define".into(), "d".into(), "word".into(), "look up".into()],
        );
        let effects = s.set_query(&format!("{keyword}  日本語 phrase"));
        assert_eq!(s.command_id(), &mode);
        assert_eq!(s.query(), "日本語 phrase");
        assert_eq!(effects[0], Effect::ReplaceQuery("日本語 phrase".into()));
        assert_eq!(searched(&effects[1..]).0, mode);
        assert_eq!(searched(&effects[1..]).2, "日本語 phrase");
        s.set_query("");
        s.intent(Intent::Backspace);
        assert_eq!(s.command_id(), &app_command());
    }
}

#[test]
fn compound_aliases_match_the_complete_keyword_and_separator() {
    let mut s = session_with_keywords(History::default(), vec!["look up".into()]);
    s.set_query("look up");
    s.intent(Intent::Space);
    assert_eq!(s.command_id(), &dictionary_command());
    s.reset();
    let effects = s.set_query("look up\t  日本語 phrase");
    assert_eq!(s.command_id(), &dictionary_command());
    assert_eq!(s.query(), "日本語 phrase");
    assert_eq!(searched(&effects[1..]).2, "日本語 phrase");
    s.reset();
    assert_eq!(searched(&s.set_query("look upper")).0, app_command());
    s.set_query("look up ");
    assert_eq!(s.command_id(), &dictionary_command());
    assert!(s.query().is_empty());
}

#[test]
fn unknown_prefixes_and_non_app_modes_keep_literal_queries() {
    for query in ["unknown word", "define", "dictionary word"] {
        let mut s = session();
        assert_eq!(searched(&s.set_query(query)).0, app_command());
        assert_eq!(s.query(), query);
    }
    let mut s = session();
    s.intent(Intent::Space);
    assert_eq!(searched(&s.set_query("d file name")).0, files_command());
    assert_eq!(s.query(), "d file name");
}

#[test]
fn a_pasted_mode_switch_discards_previous_results_and_late_deliveries() {
    let mut s = session();
    let generation = searched(&s.set_query("saf")).1;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![item("Safari")],
    });
    s.set_query("d word");
    assert!(s.items().is_empty());
    assert!(!s.deliver(Delivery::Batch {
        generation,
        items: vec![item("Safari")]
    }));
}

#[test]
fn refreshing_repeats_the_current_apps_query_with_a_new_generation() {
    let mut s = session();
    let first_effects = s.set_query("saf");
    let first = searched(&first_effects);
    let refresh_effects = s.refresh();
    let refreshed = searched(&refresh_effects);
    assert_eq!(refreshed.0, app_command());
    assert_eq!(refreshed.2, "saf");
    assert!(refreshed.1 > first.1);
}

#[test]
fn space_on_an_empty_query_enters_file_mode_and_is_consumed() {
    let mut s = session();
    assert_eq!(s.intent(Intent::Space), Some(vec![]));
    assert_eq!(s.command_id(), &files_command());
    let effects = s.set_query("report");
    assert_eq!(searched(&effects).0, files_command());
}

#[test]
fn define_then_space_enters_dictionary_mode_and_searches_the_term() {
    let mut s = session();
    s.set_query("define");
    let effects = s.intent(Intent::Space).unwrap();
    assert_eq!(s.command_id(), &dictionary_command());
    assert_eq!(s.query(), "");
    assert_eq!(effects, [Effect::ClearQuery, Effect::CancelSearch]);

    let effects = s.set_query("serendipity");
    assert_eq!(searched(&effects).0, dictionary_command());
    assert_eq!(searched(&effects).2, "serendipity");
}

#[test]
fn configured_dictionary_keywords_are_matched_as_a_list() {
    let mut s = session_with_keywords(History::default(), vec!["word".into(), "lookup".into()]);
    s.set_query("lookup");
    s.intent(Intent::Space);
    assert_eq!(s.command_id(), &dictionary_command());

    let mut s = session_with_keywords(History::default(), vec!["lookup".into()]);
    s.set_query("define");
    assert_eq!(s.intent(Intent::Space), None);
    assert_eq!(s.command_id(), &app_command());
}

#[test]
fn backspace_from_an_empty_dictionary_query_returns_to_apps() {
    let mut s = session();
    s.set_query("define");
    s.intent(Intent::Space);
    assert_eq!(s.intent(Intent::Backspace), Some(vec![]));
    assert_eq!(s.command_id(), &app_command());
}

#[test]
fn completing_a_result_keeps_the_raw_query_and_searches_it() {
    let mut s = session();
    let generation = searched(&s.set_query("g")).1;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![completion_item("Google", "g rust lifetimes")],
    });

    let effects = s.intent(Intent::Complete).unwrap();
    assert_eq!(s.query(), "g rust lifetimes");
    assert_eq!(
        effects.first(),
        Some(&Effect::ReplaceQuery("g rust lifetimes".into()))
    );
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::Search { command, query, .. }
            if command == &app_command() && query == "g rust lifetimes"
    )));
}

#[test]
fn completing_a_keyword_prefix_uses_existing_keyword_routing() {
    let mut s = session();
    let generation = searched(&s.set_query("d")).1;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![completion_item("Dictionary", "define serendipity")],
    });

    let effects = s.intent(Intent::Complete).unwrap();
    assert_eq!(s.command_id(), &dictionary_command());
    assert_eq!(s.query(), "serendipity");
    assert_eq!(effects.first(), Some(&Effect::ReplaceQuery("serendipity".into())));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::Search { command, query, .. }
            if command == &dictionary_command() && query == "serendipity"
    )));
}

#[test]
fn completion_passes_through_without_completion_or_while_rows_are_stale() {
    let mut s = session();
    let generation = searched(&s.set_query("old")).1;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![completion_item("suggestion", "new query")],
    });
    searched(&s.set_query("new"));
    assert_eq!(s.intent(Intent::Complete), None);
    assert_eq!(s.query(), "new");

    let generation = searched(&s.set_query("current")).1;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![item("ordinary result")],
    });
    assert_eq!(s.intent(Intent::Complete), None);
}

#[test]
fn enter_offers_dictionary_actions_before_launching() {
    let mut s = session();
    s.set_query("define");
    s.intent(Intent::Space);
    let generation = searched(&s.set_query("serendipity")).1;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![Item {
            id: "dictionary:serendipity".into(),
            view: View::Detail {
                title: "serendipity".into(),
                body: "The occurrence of events by chance.".into(),
            },
            record_usage: false,
            icon: None,
            matched: Vec::new(),
            completion: None,
            score: None,
            primary: None,
            alternate: None,
            actions: vec![crate::ItemAction::new(
                "dictionary",
                "Open in Dictionary",
                Action::OpenUrl("dict://serendipity".into()),
            )],
        }],
    });

    assert_eq!(
        s.intent(Intent::Open).unwrap(),
        vec![Effect::ShowActions {
            id: s.result_key(0),
            generation,
            confirmation: None,
        }]
    );
    let key = s.result_key(0);
    assert_eq!(
        s.run_key_action(&key, "dictionary")[0],
        Effect::Run(Action::OpenUrl("dict://serendipity".into()))
    );
}

#[test]
fn space_inside_a_query_is_left_to_the_field() {
    let mut s = session();
    s.set_query("visual");
    assert_eq!(s.intent(Intent::Space), None);
    assert_eq!(s.command_id(), &app_command());
}

#[test]
fn space_in_file_mode_with_an_empty_query_is_swallowed() {
    let mut s = session();
    s.intent(Intent::Space);
    assert_eq!(s.intent(Intent::Space), Some(vec![]));
    assert_eq!(s.command_id(), &files_command());
}

#[test]
fn backspace_on_an_empty_query_leaves_file_mode() {
    let mut s = session();
    s.intent(Intent::Space);
    s.set_query("a");
    // Deleting the last character is the field's job...
    assert_eq!(s.intent(Intent::Backspace), None);
    s.set_query("");
    // ...and the next Backspace, on an empty query, leaves the mode.
    assert_eq!(s.intent(Intent::Backspace), Some(vec![]));
    assert_eq!(s.command_id(), &app_command());
}

#[test]
fn backspace_on_an_empty_apps_query_is_left_to_the_field() {
    let mut s = session();
    assert_eq!(s.intent(Intent::Backspace), None);
}

#[test]
fn results_replace_on_first_batch_and_append_after() {
    let mut s = session();
    let g1 = searched(&s.set_query("s")).1;
    assert!(s.deliver(Delivery::Batch {
        generation: g1,
        items: vec![item("Safari")]
    }));
    assert!(s.deliver(Delivery::Done { generation: g1 }));
    let g2 = searched(&s.set_query("sa")).1;
    // The old rows stay until the new search answers.
    assert_eq!(s.items().len(), 1);
    assert_eq!(s.results(), Results::Searching { replaced: false });
    s.deliver(Delivery::Batch {
        generation: g2,
        items: vec![item("Safari 2")],
    });
    s.deliver(Delivery::Batch {
        generation: g2,
        items: vec![item("Safari 3")],
    });
    let titles: Vec<_> = s.items().iter().map(|i| i.display_title()).collect();
    assert_eq!(titles, ["Safari 2", "Safari 3"]);
}

#[test]
fn stale_answers_are_dropped() {
    let mut s = session();
    let g1 = searched(&s.set_query("s")).1;
    let _g2 = searched(&s.set_query("sa")).1;
    assert!(!s.deliver(Delivery::Batch {
        generation: g1,
        items: vec![item("Old")]
    }));
    assert!(s.items().is_empty());
}

#[test]
fn done_without_batches_clears_old_rows() {
    let mut s = session();
    let g1 = searched(&s.set_query("s")).1;
    s.deliver(Delivery::Batch {
        generation: g1,
        items: vec![item("Safari")],
    });
    let g2 = searched(&s.set_query("sxq")).1;
    s.deliver(Delivery::Done { generation: g2 });
    assert!(s.items().is_empty());
    assert_eq!(s.results(), Results::Done);
}

#[test]
fn clearing_the_query_cancels_and_ignores_late_answers() {
    let mut s = session();
    let g = searched(&s.set_query("s")).1;
    assert_eq!(s.set_query(""), vec![Effect::CancelSearch]);
    assert!(!s.deliver(Delivery::Batch {
        generation: g,
        items: vec![item("Late")]
    }));
    assert_eq!(s.results(), Results::Idle);
}

#[test]
fn switching_modes_never_shows_the_other_modes_rows() {
    let mut s = session();
    let g = searched(&s.set_query("s")).1;
    s.deliver(Delivery::Batch {
        generation: g,
        items: vec![item("Safari")],
    });
    s.set_query("");
    s.intent(Intent::Space);
    s.set_query("s");
    assert!(s.items().is_empty());
}

#[test]
fn arrows_wrap_and_reveal_the_selection() {
    let mut s = session();
    let g = searched(&s.set_query("a")).1;
    let items = ["A", "B", "C"].map(item).to_vec();
    s.deliver(Delivery::Batch { generation: g, items });
    assert_eq!(s.intent(Intent::Up), Some(vec![Effect::RevealSelection]));
    assert_eq!(s.selected(), 2);
    s.intent(Intent::Down);
    assert_eq!(s.selected(), 0);
}

#[test]
fn enter_runs_the_primary_action_then_hides_and_resets() {
    let mut s = session();
    let g = searched(&s.set_query("saf")).1;
    s.deliver(Delivery::Batch {
        generation: g,
        items: vec![item("Safari")],
    });
    s.deliver(Delivery::Done { generation: g });
    let effects = s.intent(Intent::Open).unwrap();
    assert_eq!(
        effects,
        vec![
            Effect::Run(Action::Open("/x/Safari".into())),
            Effect::SaveHistory,
            Effect::Hide
        ]
    );
    assert_eq!(s.query(), "");
    assert_eq!(s.command_id(), &app_command());
}

#[test]
fn command_enter_reveals_in_the_file_manager() {
    let mut s = session();
    s.intent(Intent::Space);
    let g = searched(&s.set_query("notes")).1;
    s.deliver(Delivery::Batch {
        generation: g,
        items: vec![item("notes.md")],
    });
    let effects = s.intent(Intent::OpenAlternate).unwrap();
    assert_eq!(effects[0], Effect::Run(Action::Reveal("/x/notes.md".into())));
    assert!(
        effects.contains(&Effect::CancelSearch),
        "the unfinished search stops: {effects:?}"
    );
    assert!(effects.contains(&Effect::Hide));
}

#[test]
fn enter_with_no_results_does_nothing() {
    let mut s = session();
    assert_eq!(s.intent(Intent::Open), Some(vec![]));
}

#[test]
fn escape_hides_and_resets_file_mode() {
    let mut s = session();
    s.intent(Intent::Space);
    assert_eq!(s.intent(Intent::Dismiss), Some(vec![Effect::Hide]));
    assert_eq!(s.command_id(), &app_command());
}

#[test]
fn a_click_selects_by_id_even_after_rows_move() {
    let mut s = session();
    let g = searched(&s.set_query("a")).1;
    let items = ["A", "B", "C"].map(item).to_vec();
    s.deliver(Delivery::Batch { generation: g, items });
    let key = s.result_key(2);
    assert!(s.select_key(&key));
    assert_eq!(s.selected_item().unwrap().display_title(), "C");
    let missing_key = key.replace("/x/C", "/x/gone");
    assert!(!s.select_key(&missing_key));
    assert_eq!(s.selected(), 2);
}

#[test]
fn the_in_keyword_enters_content_search_and_clears_the_field() {
    let mut s = session();
    s.set_query("in");
    let effects = s.intent(Intent::Space).unwrap();
    assert_eq!(effects.first(), Some(&Effect::ClearQuery));
    assert_eq!(s.command_id(), &contents_command());
    assert_eq!(s.query(), "");
    let effects = s.set_query("quarterly report");
    assert_eq!(searched(&effects).0, contents_command());
    assert_eq!(searched(&effects).2, "quarterly report");
}

#[test]
fn the_in_keyword_cancels_the_apps_search_it_started() {
    let mut s = session();
    s.set_query("i");
    s.set_query("in");
    assert_eq!(
        s.intent(Intent::Space),
        Some(vec![Effect::ClearQuery, Effect::CancelSearch])
    );
    assert!(s.items().is_empty());
}

#[test]
fn in_is_only_a_keyword_on_its_own_in_the_apps_search() {
    let mut s = session();
    s.set_query("ins");
    assert_eq!(s.intent(Intent::Space), None);
    s.set_query("");
    s.intent(Intent::Space);
    s.set_query("in");
    // In file search, "in " is an ordinary query.
    assert_eq!(s.intent(Intent::Space), None);
    assert_eq!(s.command_id(), &files_command());
}

#[test]
fn backspace_on_an_empty_query_leaves_content_search() {
    let mut s = session();
    s.set_query("in");
    s.intent(Intent::Space);
    assert_eq!(
        s.intent(Intent::Space),
        Some(vec![]),
        "a leading space is swallowed"
    );
    assert_eq!(s.intent(Intent::Backspace), Some(vec![]));
    assert_eq!(s.command_id(), &app_command());
}

#[test]
fn content_results_stay_on_screen_while_the_query_grows() {
    let mut s = session();
    s.set_query("in");
    s.intent(Intent::Space);
    let g = searched(&s.set_query("tax")).1;
    s.deliver(Delivery::Batch {
        generation: g,
        items: vec![item("2026.pdf")],
    });
    searched(&s.set_query("tax 2"));
    assert_eq!(s.items().len(), 1, "no flash of an empty list between keystrokes");
}

fn ranked(name: &str, score: i32) -> Item {
    Item {
        score: Some(score),
        ..item(name)
    }
}

#[test]
fn opening_a_result_teaches_the_history_and_asks_to_save_it() {
    let mut s = session_with_clock(History::default(), || 1_800_000_000);
    let g = searched(&s.set_query("s")).1;
    s.deliver(Delivery::Batch {
        generation: g,
        items: vec![ranked("Safari", 120), ranked("Shareful", 80)],
    });
    s.intent(Intent::Down);
    let effects = s.intent(Intent::Open).unwrap();
    assert_eq!(
        effects[..2],
        [
            Effect::Run(Action::Open("/x/Shareful".into())),
            Effect::SaveHistory
        ]
    );
    // The next time "s" is typed, the item picked for it comes first.
    let g = searched(&s.set_query("s")).1;
    s.deliver(Delivery::Batch {
        generation: g,
        items: vec![ranked("Safari", 120), ranked("Shareful", 80)],
    });
    assert_eq!(s.selected_item().unwrap().display_title(), "Shareful");
}

#[test]
fn reordering_on_a_later_batch_keeps_the_selected_item() {
    let mut history = History::default();
    history.record("/x/C", "a", 1_800_000_000);
    let mut s = session_with_clock(history, || 1_800_000_000);
    let g = searched(&s.set_query("a")).1;
    s.deliver(Delivery::Batch {
        generation: g,
        items: vec![ranked("A", 100), ranked("B", 90)],
    });
    s.intent(Intent::Down);
    s.deliver(Delivery::Batch {
        generation: g,
        items: vec![ranked("C", 10)],
    });
    assert_eq!(
        s.items()[0].display_title(),
        "C",
        "the item picked for this query moves to the top"
    );
    assert_eq!(
        s.selected_item().unwrap().display_title(),
        "B",
        "the selection stays on B"
    );
}

#[test]
fn refreshing_keeps_the_selected_item_when_results_reorder() {
    let mut s = session();
    let generation = searched(&s.set_query("a")).1;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![ranked("A", 100), ranked("B", 90)],
    });
    s.intent(Intent::Down);
    let generation = searched(&s.refresh()).1;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![ranked("C", 110), ranked("A", 100), ranked("B", 90)],
    });
    assert_eq!(s.selected_item().unwrap().display_title(), "B");
    let generation = searched(&s.refresh()).1;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![ranked("A", 100)],
    });
    assert_eq!(s.selected_item().unwrap().display_title(), "A");
}

#[test]
fn pasting_an_empty_keyword_cancels_the_previous_search() {
    let mut s = session();
    let generation = searched(&s.set_query("Safari")).1;
    let effects = s.set_query("d ");
    assert_eq!(s.command_id(), &dictionary_command());
    assert_eq!(s.query(), "");
    assert_eq!(
        effects,
        vec![Effect::ReplaceQuery(String::new()), Effect::CancelSearch]
    );
    assert!(!s.deliver(Delivery::Batch {
        generation,
        items: vec![ranked("Safari", 100)],
    }));
    assert!(s.items().is_empty());
}

fn complete_query(s: &mut Session, query: &str) {
    let generation = searched(&s.set_query(query)).1;
    s.deliver(Delivery::Done { generation });
}

#[test]
fn query_recall_walks_backwards_and_forwards_to_the_empty_draft() {
    let mut s = session();
    assert_eq!(s.intent(Intent::Up), Some(vec![]));
    for query in ["first", "second"] {
        complete_query(&mut s, query);
        s.set_query("");
    }
    for (intent, expected) in [
        (Intent::Up, "second"),
        (Intent::Up, "first"),
        (Intent::Up, "first"),
        (Intent::Down, "second"),
        (Intent::Down, ""),
    ] {
        s.intent(intent);
        assert_eq!(s.query(), expected);
    }
    assert!(s.search.recalled.is_none());
}

#[test]
fn empty_up_recalls_typed_query_even_when_empty_query_suggestions_exist() {
    let (mut s, default, empty) = session_with_empty_provider();
    let typed_generation = searched(&s.set_query("typed query")).1;
    s.deliver_from(
        default,
        Delivery::Done {
            generation: typed_generation,
        },
    );

    let empty_generation = searched(&s.set_query("")).1;
    s.deliver_from(
        empty.clone(),
        Delivery::Batch {
            generation: empty_generation,
            items: vec![item("suggestion")],
        },
    );
    s.deliver_from(
        empty,
        Delivery::Done {
            generation: empty_generation,
        },
    );
    assert_eq!(s.results(), Results::Done);
    assert_eq!(s.items().len(), 1);

    let effects = s.intent(Intent::Up).unwrap();
    assert_eq!(effects[0], Effect::ReplaceQuery("typed query".into()));
    assert_eq!(searched(&effects[1..]).0, app_command());
    assert_eq!(s.query(), "typed query");
}

#[test]
fn an_explicit_list_searches_every_query_and_up_wraps_on_an_empty_query() {
    let default = command("shell.launch");
    let listing = command("sample.list");
    let mut registry = CommandRegistry::new(default.clone(), None);
    registry
        .register(CommandSpec::new(default, 10, QueryPresentation::new("Launch")))
        .unwrap();
    registry
        .register(
            CommandSpec::new(listing.clone(), 10, QueryPresentation::new("Browse"))
                .with_query_eligibility(QueryEligibility::Any),
        )
        .unwrap();
    registry.add_keyword(&listing, "browse").unwrap();
    let mut s = Session::new(History::default(), registry).unwrap();
    assert!(s.refresh().is_empty());

    s.set_query("browse");
    let effects = s.intent(Intent::Space).unwrap();
    assert_eq!(effects[0], Effect::ClearQuery);
    let (provider, _, query) = searched(&effects[1..]);
    assert_eq!((provider, query), (listing.clone(), ""));

    complete_query(&mut s, "previous");
    let effects = s.set_query("");
    let (provider, generation, query) = searched(&effects);
    assert_eq!((provider, query), (listing, ""));
    s.deliver(Delivery::Batch {
        generation,
        items: vec![item("one"), item("two")],
    });
    s.deliver(Delivery::Done { generation });

    assert_eq!(s.selected(), 0);
    assert_eq!(s.intent(Intent::Up).unwrap(), [Effect::RevealSelection]);
    assert_eq!(s.selected(), 1);
    assert_eq!(s.query(), "");
    assert_eq!(s.intent(Intent::Up).unwrap(), [Effect::RevealSelection]);
    assert_eq!(s.selected(), 0);
}

#[test]
fn dictionary_words_are_recalled_without_opening_dictionary() {
    let mut s = session();
    s.set_query("d ");
    complete_query(&mut s, "serendipity");
    s.intent(Intent::Dismiss);
    s.set_query("d ");
    let effects = s.intent(Intent::Up).unwrap();
    assert_eq!(effects[0], Effect::ReplaceQuery("serendipity".into()));
    assert_eq!(searched(&effects[1..]).0, dictionary_command());
    assert_eq!(&*s.history(), &History::default());
}

#[test]
fn recall_is_separate_for_each_search_mode() {
    let mut s = session();
    complete_query(&mut s, "Safari");
    s.reset();
    s.set_query("d ");
    assert_eq!(s.intent(Intent::Up), Some(vec![]));
    complete_query(&mut s, "word");
    s.reset();
    s.intent(Intent::Up);
    assert_eq!(s.query(), "Safari");
    s.reset();
    s.set_query("d ");
    s.intent(Intent::Up);
    assert_eq!(s.query(), "word");
}

#[test]
fn editing_a_recalled_query_restores_result_navigation() {
    let mut s = session();
    complete_query(&mut s, "Safari");
    s.set_query("");
    s.intent(Intent::Up);
    let generation = searched(&s.set_query("Saf")).1;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![ranked("Safari", 100), ranked("Shareful", 90)],
    });
    assert_eq!(s.intent(Intent::Up), Some(vec![Effect::RevealSelection]));
    assert_eq!(s.selected(), 1);
    assert_eq!(s.query(), "Saf");
}

#[test]
fn recall_deduplicates_completed_terms_and_bounds_storage() {
    let mut s = session();
    for _ in 0..2 {
        complete_query(&mut s, "word");
        s.set_query("");
    }
    assert_eq!(s.queries.len(), 1);
    for i in 0..QUERY_HISTORY_LIMIT {
        complete_query(&mut s, &format!("query {i}"));
        s.set_query("");
    }
    assert_eq!(s.queries.len(), QUERY_HISTORY_LIMIT);
    assert_eq!(s.queries.front().unwrap().1, "query 0");
}

#[test]
fn unfinished_edits_are_not_recorded_but_activated_queries_are() {
    let mut s = session();
    s.set_query("unfinished");
    s.reset();
    assert_eq!(s.intent(Intent::Up), Some(vec![]));
    let generation = searched(&s.set_query("saf")).1;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![ranked("Safari", 100)],
    });
    s.intent(Intent::Open);
    s.intent(Intent::Up);
    assert_eq!(s.query(), "saf");
}

#[test]
fn clearing_one_character_at_a_time_recalls_the_completed_word() {
    let mut s = session();
    complete_query(&mut s, "abc");
    for prefix in ["ab", "a"] {
        complete_query(&mut s, prefix);
    }
    s.set_query("");
    s.intent(Intent::Up);
    assert_eq!(s.query(), "abc");
}

#[test]
fn deletion_anywhere_preserves_the_completed_term_when_cleared() {
    for (original, shortened) in [
        ("serendipity", "erendipity"),
        ("serendipity", "serendity"),
        ("日本語", "本語"),
        ("日本語", "日語"),
    ] {
        let mut s = session();
        complete_query(&mut s, original);
        complete_query(&mut s, shortened);
        s.set_query("");
        s.intent(Intent::Up);
        assert_eq!(s.query(), original);
    }
}

#[test]
fn deleting_an_unfinished_query_records_the_latest_completed_term() {
    let mut s = session();
    s.set_query("abc");
    complete_query(&mut s, "ab");
    complete_query(&mut s, "a");
    s.set_query("");
    s.intent(Intent::Up);
    assert_eq!(s.query(), "a");
}

#[test]
fn editing_after_deletion_starts_a_new_term() {
    let mut s = session();
    complete_query(&mut s, "abc");
    complete_query(&mut s, "ab");
    complete_query(&mut s, "about");
    s.set_query("");
    s.intent(Intent::Up);
    assert_eq!(s.query(), "about");
}

#[test]
fn consuming_a_keyword_does_not_add_it_to_query_history() {
    let mut s = session();
    complete_query(&mut s, "d");
    s.set_query("d word");
    s.reset();
    assert_eq!(s.intent(Intent::Up), Some(vec![]));
}

#[test]
fn switching_modes_starts_recall_from_that_modes_latest_term() {
    let mut s = session();
    complete_query(&mut s, "d");
    s.reset();
    s.set_query("d ");
    complete_query(&mut s, "word");
    s.reset();
    s.intent(Intent::Up);
    assert_eq!(s.query(), "d");
    s.intent(Intent::Space);
    assert_eq!(s.command_id(), &dictionary_command());
    s.intent(Intent::Up);
    assert_eq!(s.query(), "word");
}

#[test]
fn dismissing_after_deletion_preserves_the_completed_term() {
    for finished in [false, true] {
        for escape in [false, true] {
            let mut s = session();
            complete_query(&mut s, "serendipity");
            let generation = searched(&s.set_query("serendipit")).1;
            if finished {
                s.deliver(Delivery::Done { generation });
            }
            if escape {
                s.intent(Intent::Dismiss);
            } else {
                s.reset();
            }
            s.intent(Intent::Up);
            assert_eq!(s.query(), "serendipity");
        }
    }
}

#[test]
fn activation_after_deletion_records_only_the_activated_query() {
    for finished in [false, true] {
        let mut s = session();
        complete_query(&mut s, "safari");
        let generation = searched(&s.set_query("safar")).1;
        s.deliver(Delivery::Batch {
            generation,
            items: vec![ranked("Safari", 100)],
        });
        if finished {
            s.deliver(Delivery::Done { generation });
        }
        s.intent(Intent::Open);
        assert_eq!(s.queries, VecDeque::from([(app_command(), "safar".into())]));
    }
}

#[test]
fn dismissing_after_a_typo_correction_records_the_completed_query() {
    let mut s = session();
    s.set_query("safarx");
    complete_query(&mut s, "safar");
    s.intent(Intent::Dismiss);
    s.intent(Intent::Up);
    assert_eq!(s.query(), "safar");
}

#[test]
fn history_policy_is_independent_of_view() {
    for (view, record_usage) in [
        (
            View::Detail {
                title: "Details".into(),
                body: "Body".into(),
            },
            true,
        ),
        (
            View::Text {
                title: "Text".into(),
                subtitle: String::new(),
            },
            false,
        ),
    ] {
        let mut s = session();
        let generation = searched(&s.set_query("query")).1;
        let mut result = item("result");
        result.view = view;
        result.record_usage = record_usage;
        let id = result.id.clone();
        s.deliver(Delivery::Batch {
            generation,
            items: vec![result],
        });
        let key = s.result_key(0);
        let effects = s.run_key_action(&key, "open");
        assert!(effects.iter().any(|effect| matches!(effect, Effect::Run(_))));
        assert_eq!(effects.contains(&Effect::SaveHistory), record_usage);
        assert_eq!(s.history().boost(&id, "query", 1_800_000_000) > 0, record_usage);
    }
}

#[test]
fn non_text_actions_obey_generation_and_selection() {
    for view in [
        View::Detail {
            title: "Details".into(),
            body: "Body".into(),
        },
        View::Pair {
            source: "Source".into(),
            result: "Result".into(),
        },
    ] {
        let mut s = session();
        let generation = searched(&s.set_query("first")).1;
        let mut result = item("result");
        result.view = view;
        s.deliver(Delivery::Batch {
            generation,
            items: vec![item("text"), result.clone()],
        });
        let key = s.result_key(1);
        assert!(s.select_key(&key));
        assert_eq!(s.selected_item(), Some(&result));
        let next = searched(&s.set_query("second")).1;
        assert!(s.run_key_action(&key, "open").is_empty());
        s.deliver(Delivery::Batch {
            generation: next,
            items: vec![result],
        });
        let next_key = s.result_key(0);
        assert!(s.run_key_action(&key, "open").is_empty());
        let effects = s.run_key_action(&next_key, "open");
        assert!(effects.contains(&Effect::Run(Action::Open(PathBuf::from("/x/result")))));
    }
}

#[test]
fn snapshot_capture_rejects_inert_and_stale_results_and_restore_has_no_activation() {
    let mut s = session();
    assert!(s.capture().is_err());
    let g1 = searched(&s.set_query("first")).1;
    s.deliver(Delivery::Batch {
        generation: g1,
        items: vec![item("Historical content")],
    });
    assert!(s.capture().is_err());
    s.deliver(Delivery::Done { generation: g1 });
    let saved = s.capture().unwrap();
    assert_eq!(saved.query, "first");
    assert_eq!(saved.title, "Historical content");
    let history = s.history().to_json();
    let g2 = searched(&s.set_query("second")).1;
    assert!(s.capture().is_err());
    s.deliver(Delivery::Done { generation: g1 });
    assert!(s.capture().is_err());
    s.deliver(Delivery::Batch {
        generation: g2,
        items: vec![item("Current content")],
    });
    s.deliver(Delivery::Done { generation: g2 });
    assert_eq!(s.capture().unwrap().title, "Current content");
    let effects = s.restore(&saved).unwrap();
    assert_eq!(s.query(), "first");
    assert_eq!(s.command_id().as_str(), saved.mode);
    assert_eq!(s.history().to_json(), history);
    assert!(s.items().is_empty());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::Search { query, .. } if query == "first"))
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::Run(_) | Effect::SaveHistory))
    );
    let mut unavailable = saved.clone();
    unavailable.mode = "removed.extension".into();
    assert!(s.restore(&unavailable).is_err());
    assert!(saved.search_text().contains("historical content"));
    assert_eq!(s.query(), "first");
}

#[test]
fn restoring_an_old_default_is_explicit_under_a_new_default() {
    let mut original = session();
    let generation = searched(&original.set_query("saved")).1;
    original.deliver(Delivery::Batch {
        generation,
        items: vec![item("Historical content")],
    });
    original.deliver(Delivery::Done { generation });
    let saved = original.capture().unwrap();
    original.restore(&saved).unwrap();
    assert!(!original.search.explicit);

    let mut registry = CommandRegistry::new(files_command(), None);
    for id in [app_command(), files_command()] {
        registry
            .register(CommandSpec::new(id, 10, QueryPresentation::new("Search")))
            .unwrap();
    }
    let mut changed = Session::new(History::default(), registry).unwrap();
    let effects = changed.restore(&saved).unwrap();
    assert_eq!(changed.command_id(), &app_command());
    assert!(changed.search.explicit);
    assert_eq!(effects[0], Effect::ReplaceQuery("saved".into()));
    assert_eq!(searched(&effects[1..]).0, app_command());
}

fn rename_session() -> (Session, String) {
    let mut s = session();
    let generation = searched(&s.set_query("report")).1;
    let mut row = item("report.pdf");
    row.actions.push(ItemAction::new(
        "rename",
        "Rename…",
        Action::Rename("/x/report.pdf".into()),
    ));
    s.deliver(Delivery::Batch {
        generation,
        items: vec![item("other"), row],
    });
    s.deliver(Delivery::Done { generation });
    s.intent(Intent::Down);
    let key = s.result_key(1);
    (s, key)
}

#[test]
fn rename_puts_the_name_in_the_field_with_its_stem_selected() {
    let (mut s, key) = rename_session();
    let history = s.history().clone();
    assert_eq!(
        s.run_key_action(&key, "rename"),
        vec![
            Effect::ReplaceQuery("report.pdf".into()),
            Effect::SelectRenameStem("/x/report.pdf".into())
        ]
    );
    let renaming = s.renaming().unwrap();
    assert_eq!(renaming.path(), std::path::Path::new("/x/report.pdf"));
    assert_eq!(renaming.name, "report.pdf");
    assert_eq!(renaming.layout_key, s.layout_key(1));
    assert_eq!(s.query(), "report");
    assert_eq!(*s.history(), history);
}

fn destination_session(transfer: Transfer, settled: bool) -> (Session, u64) {
    let mut s = session();
    s.commands.set_destination_search(files_command()).unwrap();
    s.set_query("in ");
    let generation = searched(&s.set_query("report")).1;
    let path = PathBuf::from("/x/report.pdf");
    let mut source = Item::path(path.clone(), "Report".into(), vec![]);
    source.actions.push(ItemAction::new(
        "transfer",
        "Transfer",
        Action::ChooseDestination { path, transfer },
    ));
    s.deliver(Delivery::Batch {
        generation,
        items: vec![item("Other"), source.clone()],
    });
    if settled {
        s.deliver(Delivery::Done { generation });
    }
    let index = s.items().iter().position(|item| item.id == source.id).unwrap();
    let key = s.result_key(index);
    assert_eq!(
        s.run_key_action(&key, "transfer"),
        [
            Effect::CancelSearch,
            Effect::ClearQuery,
            Effect::SelectQuery(0..0)
        ]
    );
    (s, generation)
}

#[test]
fn cancelling_a_destination_restores_the_original_search_and_selection() {
    let (mut s, _) = destination_session(Transfer::Move, true);
    let history = s.history().clone();
    assert_eq!(s.destination().unwrap().transfer(), Transfer::Move);
    assert_eq!(s.destination().unwrap().path(), Path::new("/x/report.pdf"));
    assert_eq!(s.command_id(), &files_command());
    assert_eq!(s.query(), "");
    assert!(s.items().is_empty());
    assert_eq!(s.intent(Intent::Open), Some(Vec::new()));
    assert_eq!(s.intent(Intent::Backspace), None);
    let effects = s.set_query("in:~/Work Archive");
    let (command, generation, query) = searched(&effects);
    assert_eq!(command, files_command());
    assert_eq!(query, "kind:folder in:~/Work Archive");
    assert_eq!(s.query(), "in:~/Work Archive");
    assert_eq!(
        s.intent(Intent::Dismiss).unwrap(),
        [
            Effect::CancelSearch,
            Effect::ReplaceQuery("report".into()),
            Effect::RevealSelection,
        ]
    );
    assert!(!s.deliver_from(command, Delivery::Done { generation }));
    assert!(s.destination().is_none());
    assert_eq!(s.command_id(), &contents_command());
    assert_eq!(s.query(), "report");
    assert_eq!(s.results(), Results::Done);
    assert_eq!(s.items().len(), 2);
    assert_eq!(s.selected_item().unwrap().id, "/x/report.pdf");
    assert_eq!(*s.history(), history);
}

#[test]
fn enter_transfers_to_the_current_folder_and_resumes_a_pending_search() {
    let (mut s, original_generation) = destination_session(Transfer::Copy, false);
    assert!(!s.deliver_from(
        contents_command(),
        Delivery::Done {
            generation: original_generation
        }
    ));
    let (command, generation, _) = searched(&s.set_query("Archive"));
    let folder = PathBuf::from("/x/Archive");
    s.deliver_from(
        command,
        Delivery::Batch {
            generation,
            items: vec![Item::path(folder.clone(), "Archive".into(), vec![])],
        },
    );
    let key = s.result_key(0);
    assert!(s.run_key_action(&key, "open").is_empty());
    assert_eq!(s.intent(Intent::OpenAlternate), Some(Vec::new()));
    let new_effects = s.set_query("Archive kind:folder");
    let (command, generation, query) = searched(&new_effects);
    assert_eq!(query, "Archive kind:folder");
    assert_eq!(s.intent(Intent::Open), Some(Vec::new()));
    s.deliver_from(
        command,
        Delivery::Batch {
            generation,
            items: vec![Item::path(folder.clone(), "Archive".into(), vec![])],
        },
    );
    let effects = s.intent(Intent::Open).unwrap();
    assert!(matches!(&effects[..], [
        Effect::CancelSearch,
        Effect::ReplaceQuery(query),
        Effect::Search { command, generation, .. },
        Effect::RevealSelection,
        Effect::Transfer { transfer: Transfer::Copy, path, folder: destination },
    ] if query == "report" && *command == contents_command() && *generation > original_generation
        && path == Path::new("/x/report.pdf") && destination == &folder));
    assert!(s.destination().is_none());
    assert_eq!(s.selected_item().unwrap().id, "/x/report.pdf");
    assert_eq!(s.items().len(), 2);
}

#[test]
fn editing_a_name_never_searches_and_enter_renames_then_restores_the_search() {
    let (mut s, key) = rename_session();
    s.run_key_action(&key, "rename");
    assert!(s.set_query("summary.pdf").is_empty());
    assert_eq!(s.renaming().unwrap().name, "summary.pdf");
    assert_eq!(s.intent(Intent::Up), Some(Vec::new()));
    assert_eq!(s.intent(Intent::Actions), Some(Vec::new()));
    assert_eq!(s.intent(Intent::Backspace), None);
    assert_eq!(s.intent(Intent::Space), None);
    assert_eq!(s.selected(), 1);
    assert_eq!(
        s.intent(Intent::Open).unwrap(),
        vec![
            Effect::ReplaceQuery("report".into()),
            Effect::Rename {
                row: s.layout_key(1),
                path: "/x/report.pdf".into(),
                name: "summary.pdf".into(),
            }
        ]
    );
    assert!(s.renaming().is_none());
    assert_eq!(s.query(), "report");
    assert_eq!(s.items().len(), 2);
    assert_eq!(s.selected(), 1);
}

#[test]
fn enter_keeps_an_invalid_name_for_correction() {
    let (mut s, key) = rename_session();
    s.run_key_action(&key, "rename");
    for name in ["", "a/b", ".."] {
        s.set_query(name);
        assert!(s.renaming().unwrap().problem().is_some());
        assert_eq!(s.intent(Intent::Open), Some(Vec::new()));
        assert!(s.renaming().is_some());
    }
}

#[test]
fn an_unchanged_name_or_esc_leaves_without_renaming() {
    let (mut s, key) = rename_session();
    s.run_key_action(&key, "rename");
    assert_eq!(
        s.intent(Intent::Open).unwrap(),
        vec![Effect::ReplaceQuery("report".into())]
    );
    s.run_key_action(&key, "rename");
    s.set_query("changed.pdf");
    assert_eq!(
        s.intent(Intent::Dismiss).unwrap(),
        vec![Effect::ReplaceQuery("report".into())]
    );
    assert!(s.renaming().is_none());
    assert_eq!(s.selected(), 1);
    assert!(searched(&s.set_query("next")).1 > 0);
}

#[test]
fn hiding_or_selecting_elsewhere_cannot_reach_a_pending_rename() {
    let (mut s, key) = rename_session();
    s.run_key_action(&key, "rename");
    assert!(!s.select_key(&s.result_key(0)));
    assert!(s.run_key_action(&key, "rename").is_empty());
    s.reset();
    assert!(s.renaming().is_none());
    assert!(s.query().is_empty());
}

#[test]
fn a_renamed_entry_replaces_its_row_and_its_old_path_cannot_return() {
    let (mut s, _) = rename_session();
    let key = s.layout_key(1);
    let generation = s.search.generation;
    let renamed = item("summary.pdf");
    assert!(s.replace(&key, renamed.clone()));
    assert_eq!(s.items()[1].id, renamed.id);
    assert_eq!(s.selected(), 1);
    assert!(!s.replace(&key, renamed));
    s.search.generation = generation;
    s.search
        .providers
        .values_mut()
        .for_each(|p| p.phase = ProviderPhase::Pending);
    s.deliver(Delivery::Batch {
        generation,
        items: vec![item("report.pdf"), item("summary.pdf")],
    });
    assert_eq!(
        s.items().iter().map(|i| i.display_title()).collect::<Vec<_>>(),
        ["other", "summary.pdf"]
    );
}

#[test]
fn replacing_with_an_entry_already_listed_drops_the_old_row() {
    let (mut s, _) = rename_session();
    let key = s.layout_key(1);
    assert!(s.replace(&key, item("other")));
    assert_eq!(
        s.items().iter().map(|i| i.display_title()).collect::<Vec<_>>(),
        ["other"]
    );
}

#[test]
fn renaming_back_restores_the_original_row() {
    let (mut s, _) = rename_session();
    assert!(s.replace(&s.layout_key(1), item("summary.pdf")));
    let renamed = s.layout_key(1);
    assert!(s.replace(&renamed, item("report.pdf")));
    assert_eq!(
        s.items().iter().map(|i| i.display_title()).collect::<Vec<_>>(),
        ["other", "report.pdf"]
    );
}

#[test]
fn a_search_started_during_a_rename_still_receives_the_renamed_row() {
    let (mut s, key) = rename_session();
    s.run_key_action(&key, "rename");
    s.set_query("summary.pdf");
    let generation = searched(&s.refresh()).1;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![item("other"), item("report.pdf")],
    });
    let row = match s.intent(Intent::Open).unwrap().pop() {
        Some(Effect::Rename { row, .. }) => row,
        other => panic!("expected a rename, got {other:?}"),
    };
    assert!(s.replace(&row, item("summary.pdf")));
    assert_eq!(
        s.items().iter().map(|i| i.display_title()).collect::<Vec<_>>(),
        ["other", "summary.pdf"]
    );
}

#[test]
fn rename_during_refresh_updates_the_retained_row_without_reintroducing_it_as_a_match() {
    let (mut s, _) = rename_session();
    let row = s.layout_key(1);
    let generation = searched(&s.refresh()).1;
    assert!(s.replace(&row, item("summary.pdf")));
    assert_eq!(s.selected_item().unwrap().id, "/x/summary.pdf");

    s.deliver(Delivery::Batch {
        generation,
        items: vec![item("report.pdf"), item("report-archive.pdf")],
    });
    s.deliver(Delivery::Done { generation });
    assert_eq!(
        s.items().iter().map(|item| item.id.as_str()).collect::<Vec<_>>(),
        ["/x/report-archive.pdf"]
    );
}

#[test]
fn a_renamed_selection_stays_selected_when_the_refresh_finishes() {
    let (mut s, _) = rename_session();
    let row = s.layout_key(1);
    let generation = searched(&s.refresh()).1;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![item("other"), item("report.pdf")],
    });
    assert!(s.replace(&row, item("summary.pdf")));
    s.deliver(Delivery::Done { generation });
    assert_eq!(s.selected_item().unwrap().id, "/x/summary.pdf");
}

#[test]
fn enter_on_an_unchanged_name_leaves_even_when_the_name_is_now_invalid() {
    let mut s = session();
    let generation = searched(&s.set_query("long")).1;
    let name = "あ".repeat(256);
    let mut row = item(&name);
    row.actions.push(ItemAction::new(
        "rename",
        "Rename…",
        Action::Rename(format!("/x/{name}").into()),
    ));
    s.deliver(Delivery::Batch {
        generation,
        items: vec![row],
    });
    let key = s.result_key(0);
    s.run_key_action(&key, "rename");
    assert!(s.renaming().unwrap().problem().is_some());
    assert_eq!(
        s.intent(Intent::Open).unwrap(),
        vec![Effect::ReplaceQuery("long".into())]
    );
}

fn folder_item(path: &str) -> Item {
    let mut item = Item::path(path.into(), path.rsplit('/').next().unwrap().into(), vec![]);
    item.actions
        .push(ItemAction::new("browse", "Browse", Action::Browse(path.into())));
    item
}

fn listed(effects: &[Effect]) -> (u64, String, Option<PathBuf>) {
    match effects
        .iter()
        .find(|effect| matches!(effect, Effect::Search { .. }))
    {
        Some(Effect::Search {
            command,
            generation,
            query,
            folder,
        }) if *command == browse_command() => (*generation, query.clone(), folder.clone()),
        other => panic!("expected a folder listing, got {other:?}"),
    }
}

/// File search for "doc", with the folder `/x/docs` selected below a file.
fn searching_docs() -> Session {
    let mut s = session();
    s.intent(Intent::Space);
    let generation = searched(&s.set_query("doc")).1;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![item("doc.txt"), folder_item("/x/docs")],
    });
    s.intent(Intent::Down);
    s
}

#[test]
fn tab_on_a_folder_lists_it_and_backspace_restores_the_search_and_selection() {
    let mut s = searching_docs();
    let effects = s.intent(Intent::Complete).unwrap();
    assert_eq!(effects[0], Effect::ClearQuery);
    let (generation, query, folder) = listed(&effects);
    assert_eq!((query.as_str(), folder), ("", Some("/x/docs".into())));
    assert_eq!(s.folder(), Some(std::path::Path::new("/x/docs")));
    assert!(
        s.items().is_empty(),
        "the search's rows never show as the folder's"
    );
    s.deliver(Delivery::Batch {
        generation,
        items: vec![item("a"), item("b")],
    });
    assert_eq!(s.intent(Intent::Up), Some(vec![Effect::RevealSelection]));
    assert_eq!(s.selected_item().unwrap().display_title(), "b");

    let (_, query, folder) = listed(&s.set_query("a"));
    assert_eq!((query.as_str(), folder), ("a", Some("/x/docs".into())));
    assert_eq!(
        s.intent(Intent::Backspace),
        None,
        "a filter is edited in the field"
    );
    listed(&s.set_query(""));

    let effects = s.intent(Intent::Backspace).unwrap();
    assert_eq!(effects[0], Effect::ReplaceQuery("doc".into()));
    let (command, generation, query) = searched(&effects[1..]);
    assert_eq!((command, query), (files_command(), "doc"));
    assert_eq!(s.folder(), None);
    s.deliver(Delivery::Batch {
        generation,
        items: vec![item("other"), folder_item("/x/docs"), item("doc.txt")],
    });
    assert_eq!(s.selected_item().unwrap().id, "/x/docs");
    assert_eq!(s.command_id(), &files_command());
}

#[test]
fn nested_folders_return_to_their_parent_filter_and_selection() {
    let mut s = searching_docs();
    let generation = listed(&s.intent(Intent::Complete).unwrap()).0;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![folder_item("/x/docs/sub")],
    });
    let generation = listed(&s.set_query("su")).0;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![folder_item("/x/docs/sub")],
    });
    let (_, query, folder) = listed(&s.intent(Intent::Complete).unwrap());
    assert_eq!((query.as_str(), folder), ("", Some("/x/docs/sub".into())));

    let effects = s.intent(Intent::Backspace).unwrap();
    assert_eq!(effects[0], Effect::ReplaceQuery("su".into()));
    let (generation, query, folder) = listed(&effects);
    assert_eq!((query.as_str(), folder), ("su", Some("/x/docs".into())));
    s.deliver(Delivery::Batch {
        generation,
        items: vec![item("summary"), folder_item("/x/docs/sub")],
    });
    assert_eq!(s.selected_item().unwrap().id, "/x/docs/sub");
}

#[test]
fn the_browse_action_enters_a_folder_and_dismissing_forgets_it() {
    let mut s = searching_docs();
    let key = s.result_key(1);
    let generation = listed(&s.run_key_action(&key, "browse")).0;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![item("a")],
    });
    s.deliver(Delivery::Done { generation });
    let saved = s.capture().unwrap();
    assert!(
        s.restore(&saved).is_err(),
        "a listing needs the folder it came from"
    );
    assert_eq!(s.folder(), Some(std::path::Path::new("/x/docs")));

    s.intent(Intent::Dismiss);
    assert_eq!(s.folder(), None);
    assert_eq!(s.command_id(), &app_command());
    assert_eq!(
        s.intent(Intent::Up),
        Some(Vec::new()),
        "browsing records no queries"
    );
}
