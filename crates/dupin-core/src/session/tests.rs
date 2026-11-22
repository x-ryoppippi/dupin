use std::path::PathBuf;

use super::*;
use crate::ItemKind;

fn item(kind: ItemKind, name: &str) -> Item {
    Item::path(kind, PathBuf::from(format!("/x/{name}")), name.to_owned(), vec![])
}

fn searched(effects: &[Effect]) -> (Mode, u64, &str) {
    match effects {
        [
            Effect::Search {
                mode,
                generation,
                query,
            },
        ] => (*mode, *generation, query),
        other => panic!("expected one search, got {other:?}"),
    }
}

#[test]
fn starts_in_apps_mode_with_nothing_shown() {
    let s = Session::default();
    assert_eq!(s.mode(), Mode::Apps);
    assert_eq!(s.results(), Results::Idle);
    assert!(s.items().is_empty());
}

#[test]
fn opening_a_dictionary_result_does_not_change_ranking_history() {
    let mut history = History::default();
    history.record("/Applications/Safari.app", "saf", 1_800_000_000);
    let mut s = Session::new(history.clone());
    s.set_query("d");
    s.intent(Intent::Space);
    let generation = searched(&s.set_query("serendipity")).1;
    let mut result = item(ItemKind::Dictionary, "serendipity");
    let action = Action::OpenUrl("dict://serendipity".into());
    result.primary = action.clone();
    s.deliver(Delivery::Batch {
        generation,
        items: vec![result],
    });
    let effects = s.intent(Intent::Open).unwrap();
    assert!(effects.contains(&Effect::Run(action)));
    assert!(!effects.contains(&Effect::SaveHistory));
    assert_eq!(s.history(), &history);
}

#[test]
fn typing_searches_apps() {
    let mut s = Session::default();
    let effects = s.set_query("saf");
    assert_eq!(searched(&effects).0, Mode::Apps);
    assert_eq!(searched(&effects).2, "saf");
}

#[test]
fn pasted_keywords_use_the_same_modes_and_aliases_as_typed_keywords() {
    for (keyword, mode) in [
        ("in", Mode::Contents),
        ("define", Mode::Dictionary),
        ("d", Mode::Dictionary),
        ("word", Mode::Dictionary),
        ("look up", Mode::Dictionary),
    ] {
        let mut s = Session::with_dictionary_keywords(
            History::default(),
            vec!["define".into(), "d".into(), "word".into(), "look up".into()],
        );
        let effects = s.set_query(&format!("{keyword}  日本語 phrase"));
        assert_eq!(s.mode(), mode);
        assert_eq!(s.query(), "日本語 phrase");
        assert_eq!(effects[0], Effect::ReplaceQuery("日本語 phrase".into()));
        assert_eq!(searched(&effects[1..]).0, mode);
        assert_eq!(searched(&effects[1..]).2, "日本語 phrase");
        s.set_query("");
        s.intent(Intent::Backspace);
        assert_eq!(s.mode(), Mode::Apps);
    }
}

#[test]
fn compound_aliases_match_the_complete_keyword_and_separator() {
    let mut s = Session::with_dictionary_keywords(History::default(), vec!["look up".into()]);
    s.set_query("look up");
    s.intent(Intent::Space);
    assert_eq!(s.mode(), Mode::Dictionary);
    s.reset();
    let effects = s.set_query("look up\t  日本語 phrase");
    assert_eq!(s.mode(), Mode::Dictionary);
    assert_eq!(s.query(), "日本語 phrase");
    assert_eq!(searched(&effects[1..]).2, "日本語 phrase");
    s.reset();
    assert_eq!(searched(&s.set_query("look upper")).0, Mode::Apps);
    s.set_query("look up ");
    assert_eq!(s.mode(), Mode::Dictionary);
    assert!(s.query().is_empty());
}

#[test]
fn unknown_prefixes_and_non_app_modes_keep_literal_queries() {
    for query in ["unknown word", "define", "dictionary word"] {
        let mut s = Session::default();
        assert_eq!(searched(&s.set_query(query)).0, Mode::Apps);
        assert_eq!(s.query(), query);
    }
    let mut s = Session::default();
    s.intent(Intent::Space);
    assert_eq!(searched(&s.set_query("d file name")).0, Mode::Files);
    assert_eq!(s.query(), "d file name");
}

#[test]
fn a_pasted_mode_switch_discards_previous_results_and_late_deliveries() {
    let mut s = Session::default();
    let generation = searched(&s.set_query("saf")).1;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![item(ItemKind::Application, "Safari")],
    });
    s.set_query("d word");
    assert!(s.items().is_empty());
    assert!(!s.deliver(Delivery::Batch {
        generation,
        items: vec![item(ItemKind::Application, "Safari")]
    }));
}

#[test]
fn refreshing_repeats_the_current_apps_query_with_a_new_generation() {
    let mut s = Session::default();
    let first_effects = s.set_query("saf");
    let first = searched(&first_effects);
    let refresh_effects = s.refresh();
    let refreshed = searched(&refresh_effects);
    assert_eq!(refreshed.0, Mode::Apps);
    assert_eq!(refreshed.2, "saf");
    assert!(refreshed.1 > first.1);
}

#[test]
fn space_on_an_empty_query_enters_file_mode_and_is_consumed() {
    let mut s = Session::default();
    assert_eq!(s.intent(Intent::Space), Some(vec![]));
    assert_eq!(s.mode(), Mode::Files);
    let effects = s.set_query("report");
    assert_eq!(searched(&effects).0, Mode::Files);
}

#[test]
fn define_then_space_enters_dictionary_mode_and_searches_the_term() {
    let mut s = Session::default();
    s.set_query("define");
    let effects = s.intent(Intent::Space).unwrap();
    assert_eq!(s.mode(), Mode::Dictionary);
    assert_eq!(s.query(), "");
    assert_eq!(effects, [Effect::ClearQuery, Effect::CancelSearch]);

    let effects = s.set_query("serendipity");
    assert_eq!(searched(&effects).0, Mode::Dictionary);
    assert_eq!(searched(&effects).2, "serendipity");
}

#[test]
fn configured_dictionary_keywords_are_matched_as_a_list() {
    let mut s = Session::with_dictionary_keywords(History::default(), vec!["word".into(), "lookup".into()]);
    s.set_query("lookup");
    s.intent(Intent::Space);
    assert_eq!(s.mode(), Mode::Dictionary);

    let mut s = Session::with_dictionary_keywords(History::default(), vec!["lookup".into()]);
    s.set_query("define");
    assert_eq!(s.intent(Intent::Space), None);
    assert_eq!(s.mode(), Mode::Apps);
}

#[test]
fn backspace_from_an_empty_dictionary_query_returns_to_apps() {
    let mut s = Session::default();
    s.set_query("define");
    s.intent(Intent::Space);
    assert_eq!(s.intent(Intent::Backspace), Some(vec![]));
    assert_eq!(s.mode(), Mode::Apps);
}

#[test]
fn enter_opens_the_selected_dictionary_term() {
    let mut s = Session::default();
    s.set_query("define");
    s.intent(Intent::Space);
    let generation = searched(&s.set_query("serendipity")).1;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![Item {
            id: "dictionary:serendipity".into(),
            title: "serendipity".into(),
            subtitle: "The occurrence of events by chance.".into(),
            kind: ItemKind::Dictionary,
            icon: None,
            matched: Vec::new(),
            score: None,
            primary: Action::OpenUrl("dict://serendipity".into()),
            alternate: None,
        }],
    });

    assert_eq!(
        s.intent(Intent::Open).unwrap()[0],
        Effect::Run(Action::OpenUrl("dict://serendipity".into()))
    );
}

#[test]
fn space_inside_a_query_is_left_to_the_field() {
    let mut s = Session::default();
    s.set_query("visual");
    assert_eq!(s.intent(Intent::Space), None);
    assert_eq!(s.mode(), Mode::Apps);
}

#[test]
fn space_in_file_mode_with_an_empty_query_is_swallowed() {
    let mut s = Session::default();
    s.intent(Intent::Space);
    assert_eq!(s.intent(Intent::Space), Some(vec![]));
    assert_eq!(s.mode(), Mode::Files);
}

#[test]
fn backspace_on_an_empty_query_leaves_file_mode() {
    let mut s = Session::default();
    s.intent(Intent::Space);
    s.set_query("a");
    // Deleting the last character is the field's job...
    assert_eq!(s.intent(Intent::Backspace), None);
    s.set_query("");
    // ...and the next Backspace, on an empty query, leaves the mode.
    assert_eq!(s.intent(Intent::Backspace), Some(vec![]));
    assert_eq!(s.mode(), Mode::Apps);
}

#[test]
fn backspace_on_an_empty_apps_query_is_left_to_the_field() {
    let mut s = Session::default();
    assert_eq!(s.intent(Intent::Backspace), None);
}

#[test]
fn results_replace_on_first_batch_and_append_after() {
    let mut s = Session::default();
    let g1 = searched(&s.set_query("s")).1;
    assert!(s.deliver(Delivery::Batch {
        generation: g1,
        items: vec![item(ItemKind::Application, "Safari")]
    }));
    assert!(s.deliver(Delivery::Done { generation: g1 }));
    let g2 = searched(&s.set_query("sa")).1;
    // The old rows stay until the new search answers.
    assert_eq!(s.items().len(), 1);
    assert_eq!(s.results(), Results::Searching { replaced: false });
    s.deliver(Delivery::Batch {
        generation: g2,
        items: vec![item(ItemKind::Application, "Safari 2")],
    });
    s.deliver(Delivery::Batch {
        generation: g2,
        items: vec![item(ItemKind::Application, "Safari 3")],
    });
    let titles: Vec<_> = s.items().iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, ["Safari 2", "Safari 3"]);
}

#[test]
fn stale_answers_are_dropped() {
    let mut s = Session::default();
    let g1 = searched(&s.set_query("s")).1;
    let _g2 = searched(&s.set_query("sa")).1;
    assert!(!s.deliver(Delivery::Batch {
        generation: g1,
        items: vec![item(ItemKind::Application, "Old")]
    }));
    assert!(s.items().is_empty());
}

#[test]
fn done_without_batches_clears_old_rows() {
    let mut s = Session::default();
    let g1 = searched(&s.set_query("s")).1;
    s.deliver(Delivery::Batch {
        generation: g1,
        items: vec![item(ItemKind::Application, "Safari")],
    });
    let g2 = searched(&s.set_query("sxq")).1;
    s.deliver(Delivery::Done { generation: g2 });
    assert!(s.items().is_empty());
    assert_eq!(s.results(), Results::Done);
}

#[test]
fn clearing_the_query_cancels_and_ignores_late_answers() {
    let mut s = Session::default();
    let g = searched(&s.set_query("s")).1;
    assert_eq!(s.set_query(""), vec![Effect::CancelSearch]);
    assert!(!s.deliver(Delivery::Batch {
        generation: g,
        items: vec![item(ItemKind::Application, "Late")]
    }));
    assert_eq!(s.results(), Results::Idle);
}

#[test]
fn switching_modes_never_shows_the_other_modes_rows() {
    let mut s = Session::default();
    let g = searched(&s.set_query("s")).1;
    s.deliver(Delivery::Batch {
        generation: g,
        items: vec![item(ItemKind::Application, "Safari")],
    });
    s.set_query("");
    s.intent(Intent::Space);
    s.set_query("s");
    assert!(s.items().is_empty());
}

#[test]
fn arrows_wrap_and_reveal_the_selection() {
    let mut s = Session::default();
    let g = searched(&s.set_query("a")).1;
    let items = ["A", "B", "C"].map(|n| item(ItemKind::Application, n)).to_vec();
    s.deliver(Delivery::Batch { generation: g, items });
    assert_eq!(s.intent(Intent::Up), Some(vec![Effect::RevealSelection]));
    assert_eq!(s.selected(), 2);
    s.intent(Intent::Down);
    assert_eq!(s.selected(), 0);
}

#[test]
fn enter_runs_the_primary_action_then_hides_and_resets() {
    let mut s = Session::default();
    let g = searched(&s.set_query("saf")).1;
    s.deliver(Delivery::Batch {
        generation: g,
        items: vec![item(ItemKind::Application, "Safari")],
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
    assert_eq!(s.mode(), Mode::Apps);
}

#[test]
fn command_enter_reveals_in_the_file_manager() {
    let mut s = Session::default();
    s.intent(Intent::Space);
    let g = searched(&s.set_query("notes")).1;
    s.deliver(Delivery::Batch {
        generation: g,
        items: vec![item(ItemKind::File, "notes.md")],
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
    let mut s = Session::default();
    assert_eq!(s.intent(Intent::Open), Some(vec![]));
}

#[test]
fn escape_hides_and_resets_file_mode() {
    let mut s = Session::default();
    s.intent(Intent::Space);
    assert_eq!(s.intent(Intent::Dismiss), Some(vec![Effect::Hide]));
    assert_eq!(s.mode(), Mode::Apps);
}

#[test]
fn a_click_selects_by_id_even_after_rows_move() {
    let mut s = Session::default();
    let g = searched(&s.set_query("a")).1;
    let items = ["A", "B", "C"].map(|n| item(ItemKind::Application, n)).to_vec();
    s.deliver(Delivery::Batch { generation: g, items });
    assert!(s.select_id("/x/C"));
    assert_eq!(s.selected_item().unwrap().title, "C");
    assert!(!s.select_id("/x/gone"));
    assert_eq!(s.selected(), 2);
}

#[test]
fn the_in_keyword_enters_content_search_and_clears_the_field() {
    let mut s = Session::default();
    s.set_query("in");
    let effects = s.intent(Intent::Space).unwrap();
    assert_eq!(effects.first(), Some(&Effect::ClearQuery));
    assert_eq!(s.mode(), Mode::Contents);
    assert_eq!(s.query(), "");
    let effects = s.set_query("quarterly report");
    assert_eq!(searched(&effects).0, Mode::Contents);
    assert_eq!(searched(&effects).2, "quarterly report");
}

#[test]
fn the_in_keyword_cancels_the_apps_search_it_started() {
    let mut s = Session::default();
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
    let mut s = Session::default();
    s.set_query("ins");
    assert_eq!(s.intent(Intent::Space), None);
    s.set_query("");
    s.intent(Intent::Space);
    s.set_query("in");
    // In file search, "in " is an ordinary query.
    assert_eq!(s.intent(Intent::Space), None);
    assert_eq!(s.mode(), Mode::Files);
}

#[test]
fn backspace_on_an_empty_query_leaves_content_search() {
    let mut s = Session::default();
    s.set_query("in");
    s.intent(Intent::Space);
    assert_eq!(
        s.intent(Intent::Space),
        Some(vec![]),
        "a leading space is swallowed"
    );
    assert_eq!(s.intent(Intent::Backspace), Some(vec![]));
    assert_eq!(s.mode(), Mode::Apps);
}

#[test]
fn content_results_stay_on_screen_while_the_query_grows() {
    let mut s = Session::default();
    s.set_query("in");
    s.intent(Intent::Space);
    let g = searched(&s.set_query("tax")).1;
    s.deliver(Delivery::Batch {
        generation: g,
        items: vec![item(ItemKind::File, "2026.pdf")],
    });
    searched(&s.set_query("tax 2"));
    assert_eq!(s.items().len(), 1, "no flash of an empty list between keystrokes");
}

fn ranked(name: &str, score: i32) -> Item {
    Item {
        score: Some(score),
        ..item(ItemKind::Application, name)
    }
}

#[test]
fn opening_a_result_teaches_the_history_and_asks_to_save_it() {
    let mut s = Session::with_clock(History::default(), || 1_800_000_000);
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
    assert_eq!(s.selected_item().unwrap().title, "Shareful");
}

#[test]
fn reordering_on_a_later_batch_keeps_the_selected_item() {
    let mut history = History::default();
    history.record("/x/C", "a", 1_800_000_000);
    let mut s = Session::with_clock(history, || 1_800_000_000);
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
        s.items()[0].title,
        "C",
        "the item picked for this query moves to the top"
    );
    assert_eq!(s.selected_item().unwrap().title, "B", "the selection stays on B");
}

#[test]
fn refreshing_keeps_the_selected_item_when_results_reorder() {
    let mut s = Session::default();
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
    assert_eq!(s.selected_item().unwrap().title, "B");
    let generation = searched(&s.refresh()).1;
    s.deliver(Delivery::Batch {
        generation,
        items: vec![ranked("A", 100)],
    });
    assert_eq!(s.selected_item().unwrap().title, "A");
}

#[test]
fn pasting_an_empty_keyword_cancels_the_previous_search() {
    let mut s = Session::default();
    let generation = searched(&s.set_query("Safari")).1;
    let effects = s.set_query("d ");
    assert_eq!(s.mode(), Mode::Dictionary);
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
    let mut s = Session::default();
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
    let mut s = Session::default();
    s.set_query("d ");
    complete_query(&mut s, "serendipity");
    s.intent(Intent::Dismiss);
    s.set_query("d ");
    let effects = s.intent(Intent::Up).unwrap();
    assert_eq!(effects[0], Effect::ReplaceQuery("serendipity".into()));
    assert_eq!(searched(&effects[1..]).0, Mode::Dictionary);
    assert_eq!(s.history(), &History::default());
}

#[test]
fn recall_is_separate_for_each_search_mode() {
    let mut s = Session::default();
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
    let mut s = Session::default();
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
    let mut s = Session::default();
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
    let mut s = Session::default();
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
    let mut s = Session::default();
    complete_query(&mut s, "abc");
    for prefix in ["ab", "a"] {
        complete_query(&mut s, prefix);
    }
    s.set_query("");
    s.intent(Intent::Up);
    assert_eq!(s.query(), "abc");
}

#[test]
fn deleting_an_unfinished_query_does_not_save_its_completed_fragments() {
    let mut s = Session::default();
    s.set_query("abc");
    complete_query(&mut s, "ab");
    complete_query(&mut s, "a");
    s.set_query("");
    assert_eq!(s.intent(Intent::Up), Some(vec![]));
}

#[test]
fn editing_after_deletion_starts_a_new_term() {
    let mut s = Session::default();
    complete_query(&mut s, "abc");
    complete_query(&mut s, "ab");
    complete_query(&mut s, "about");
    s.set_query("");
    s.intent(Intent::Up);
    assert_eq!(s.query(), "about");
}

#[test]
fn consuming_a_keyword_does_not_add_it_to_query_history() {
    let mut s = Session::default();
    complete_query(&mut s, "d");
    s.set_query("d word");
    s.reset();
    assert_eq!(s.intent(Intent::Up), Some(vec![]));
}
