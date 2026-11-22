use std::path::PathBuf;

use super::*;
use crate::{CommandId, CommandRegistry, CommandSpec, QueryPresentation, View};

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
            },
        ] => (command.clone(), *generation, query),
        other => panic!("expected one search, got {other:?}"),
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
            generation
        }]
    );
    s.intent(Intent::Down);
    let effects = s.run_key_action(&key, "copy-path");
    assert_eq!(effects[0], Effect::Run(Action::Copy("/x/notes.md".into())));
    assert!(effects.contains(&Effect::Hide));
    assert!(s.items().is_empty());
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
        assert_eq!(s.history(), &History::default());
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
    assert_eq!(s.history(), &history);
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
            generation
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
    assert_eq!(effects.last(), Some(&Effect::Hide));
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
    assert!(s.recalled.is_none());
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
    assert_eq!(s.history(), &History::default());
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
