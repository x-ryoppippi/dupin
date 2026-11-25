use std::path::PathBuf;

use super::*;
use crate::searcher::Delivery;
use crate::{CommandRegistry, CommandSpec, Effect, History, Intent, QueryPresentation};

fn id(value: &str) -> CommandId {
    CommandId::new(value).unwrap()
}

fn registry() -> CommandRegistry {
    let mut registry = CommandRegistry::new(id("apps.search"), Some(id("files.names")));
    for (command, chip, placeholder) in [
        ("files.names", "Files", "Search files"),
        ("calendar.events", "Calendar", "Search calendar events"),
        ("clipboard.history", "Clipboard", "Search clipboard history"),
        ("emoji.pick", "Emoji", "Pick an emoji"),
    ] {
        let mut presentation = QueryPresentation::new(placeholder);
        presentation.chip = Some(chip.into());
        registry
            .register(
                CommandSpec::new(id(command), 10, presentation)
                    .with_query_eligibility(crate::QueryEligibility::Any),
            )
            .unwrap();
    }
    registry
        .register(
            CommandSpec::new(
                id("dictionary.lookup"),
                10,
                QueryPresentation::new("Look up a word"),
            )
            .with_title("Dictionary"),
        )
        .unwrap();
    registry
        .register(CommandSpec::new(
            id("apps.search"),
            10,
            QueryPresentation::new("Search apps"),
        ))
        .unwrap();
    for (command, keyword) in [
        ("calendar.events", "cal"),
        ("clipboard.history", "clip"),
        ("emoji.pick", "emoji"),
        ("dictionary.lookup", "define"),
        ("dictionary.lookup", "d"),
    ] {
        registry.add_keyword(&id(command), keyword).unwrap();
    }
    registry.add_web_alias("g", "Search Google").unwrap();
    registry.add_web_alias("gh", "Search GitHub").unwrap();
    registry
}

fn app(name: &str) -> Item {
    Item::path(
        PathBuf::from(format!("/Applications/{name}.app")),
        name.into(),
        Vec::new(),
    )
}

fn session() -> Session {
    Session::new(History::default(), registry()).unwrap()
}

type Shown = Vec<(String, Vec<String>)>;

/// The main search for `query` once apps answered with `apps`: each row's title and tags.
fn shown(session: &mut Session, query: &str, apps: &[&str]) -> Shown {
    let generation = session
        .set_query(query)
        .iter()
        .find_map(|effect| match effect {
            Effect::Search { generation, .. } => Some(*generation),
            _ => None,
        })
        .unwrap();
    session.deliver_from(
        id("apps.search"),
        Delivery::Batch {
            generation,
            items: apps.iter().map(|name| app(name)).collect(),
        },
    );
    session.deliver_from(id("apps.search"), Delivery::Done { generation });
    (0..session.items().len())
        .map(|index| {
            let tags = session
                .accessories(index)
                .iter()
                .map(|accessory| match accessory {
                    Accessory::Tag(tag) | Accessory::Text(tag) => tag.clone(),
                })
                .collect();
            (session.items()[index].display_title().to_owned(), tags)
        })
        .collect()
}

fn titles(rows: &Shown) -> Vec<&str> {
    rows.iter().map(|(title, _)| title.as_str()).collect()
}

#[test]
fn an_exact_keyword_leads_and_the_start_of_one_follows_the_best_app() {
    let mut s = session();
    let rows = shown(&mut s, "cal", &["Calculator", "Calendar"]);
    assert_eq!(titles(&rows), ["Calendar", "Calculator", "Calendar"]);
    assert_eq!(rows[0].1, ["cal ␣"]);
    assert_eq!(s.origins()[0].as_str(), KEYWORDS);

    let rows = shown(&mut s, "ca", &["Calculator", "Calendar"]);
    assert_eq!(titles(&rows), ["Calculator", "Calendar", "Calendar"]);
    assert_eq!(s.origins()[1].as_str(), KEYWORDS);

    // `c` starts two commands' keywords and is neither, so both follow the best app.
    let rows = shown(&mut s, "c", &["Chess", "Calculator"]);
    assert_eq!(titles(&rows), ["Chess", "Calendar", "Clipboard", "Calculator"]);

    // A letter that only starts a keyword keeps the app first, even the only command it starts.
    assert_eq!(titles(&shown(&mut s, "e", &["Excel"])), ["Excel", "Emoji"]);

    // A short keyword leads when no other command's keyword starts with it.
    let rows = shown(&mut s, "d", &["Discord"]);
    assert_eq!(titles(&rows), ["Dictionary", "Discord"]);
    assert_eq!(rows[0].1, ["d ␣", "define ␣"]);
    let rows = shown(&mut s, "DEFINE", &["Default Folder"]);
    assert_eq!(titles(&rows), ["Dictionary", "Default Folder"]);
    assert_eq!(rows[0].1, ["define ␣", "d ␣"]);

    // With nothing else found, the commands are the results.
    assert_eq!(titles(&shown(&mut s, "emo", &[])), ["Emoji"]);
    assert_eq!(titles(&shown(&mut s, "safari", &["Safari"])), ["Safari"]);
}

#[test]
fn a_web_alias_adds_no_second_row_for_itself() {
    let mut s = session();
    let rows = shown(&mut s, "g", &["Grapher"]);
    assert_eq!(titles(&rows), ["Grapher", "Search GitHub"]);
    assert_eq!(rows[1].1, ["gh ␣"]);
    assert_eq!(
        titles(&shown(&mut s, "gh", &["GitHub Desktop"])),
        ["GitHub Desktop"]
    );
}

#[test]
fn enter_and_tab_on_a_command_row_type_its_keyword_and_space() {
    for intent in [Intent::Open, Intent::Complete] {
        let mut s = session();
        shown(&mut s, "ca", &["Calculator"]);
        s.intent(Intent::Down).unwrap();
        let effects = s.intent(intent).unwrap();
        assert!(
            effects.contains(&Effect::ReplaceQuery(String::new())),
            "{effects:?}"
        );
        assert!(effects.iter().any(|effect| matches!(
            effect,
            Effect::Search { command, query, .. } if command.as_str() == "calendar.events" && query.is_empty()
        )));
        assert_eq!(s.command_id().as_str(), "calendar.events");
        assert!(!effects.contains(&Effect::SaveHistory));

        // A web search's alias stays in the query, as typing it and Space leaves it.
        let mut s = session();
        shown(&mut s, "g", &["Grapher"]);
        s.intent(Intent::Down).unwrap();
        let effects = s.intent(intent).unwrap();
        assert_eq!(effects[0], Effect::ReplaceQuery("gh ".into()));
        assert_eq!(s.query(), "gh ");
        assert_eq!(s.command_id().as_str(), "apps.search");
    }
}

#[test]
fn a_question_mark_lists_every_command_and_its_keywords() {
    let mut s = session();
    let rows = shown(&mut s, "?", &[]);
    let expected: Shown = [
        ("Calendar", &["cal ␣"][..]),
        ("Clipboard", &["clip ␣"]),
        ("Dictionary", &["d ␣", "define ␣"]),
        ("Emoji", &["emoji ␣"]),
        ("Files", &["Space"]),
        ("Search GitHub", &["gh ␣"]),
        ("Search Google", &["g ␣"]),
    ]
    .iter()
    .map(|(title, tags)| {
        (
            (*title).to_owned(),
            tags.iter().map(|tag| (*tag).to_owned()).collect(),
        )
    })
    .collect();
    assert_eq!(rows, expected);
    // Files is what Space on an empty query enters.
    while s.selected_item().unwrap().display_title() != "Files" {
        s.intent(Intent::Down).unwrap();
    }
    let effects = s.intent(Intent::Open).unwrap();
    assert_eq!(effects[0], Effect::ClearQuery);
    assert_eq!(s.command_id().as_str(), "files.names");
}

#[test]
fn only_a_query_in_the_main_search_suggests() {
    let mut s = session();
    s.refresh();
    assert!(s.items().is_empty());
    let mut s = session();
    assert!(titles(&shown(&mut s, "cal x", &[])).is_empty());

    // Under a command, its keyword is just text.
    let mut s = session();
    s.set_query("cal ");
    assert_eq!(s.command_id().as_str(), "calendar.events");
    let generation = match s.set_query("clip").as_slice() {
        [Effect::Search { generation, .. }] => *generation,
        effects => panic!("{effects:?}"),
    };
    s.deliver_from(id("calendar.events"), Delivery::Done { generation });
    assert!(s.items().is_empty());
}

#[test]
fn the_registry_reports_who_holds_a_keyword() {
    let mut registry = registry();
    assert_eq!(
        registry.holder("d"),
        Some(crate::KeywordHolder::Command(&id("dictionary.lookup")))
    );
    assert_eq!(
        registry.holder("gh"),
        Some(crate::KeywordHolder::Web("Search GitHub"))
    );
    assert_eq!(registry.holder("yt"), None);
    assert!(registry.add_keyword(&id("emoji.pick"), "g").is_err());
    assert!(registry.add_web_alias("cal", "Calendar search").is_err());
    assert!(registry.add_web_alias("g", "Again").is_err());
    assert!(registry.keyword("g").is_none());
}

#[test]
fn an_app_alias_leads_with_its_app_shown_once() {
    let mut registry = registry();
    registry
        .add_path_alias("ff", PathBuf::from("/Applications/Firefox.app"), "Firefox")
        .unwrap();
    registry.add_web_shorthand("goo", "g").unwrap();
    assert!(
        registry.add_web_shorthand("x", "goo").is_err(),
        "a shorthand of a shorthand"
    );
    let mut s = Session::new(History::default(), registry).unwrap();
    let rows = shown(&mut s, "ff", &["Fluffy", "Firefox"]);
    assert_eq!(titles(&rows), ["Firefox", "Fluffy"]);
    assert_eq!(rows[0].1, ["ff"]);
    assert_eq!(s.origins()[0].as_str(), KEYWORDS);
    let firefox = s.selected_item().unwrap();
    assert_eq!(
        firefox.action(firefox.primary.as_deref().unwrap()),
        Some(&Action::Open("/Applications/Firefox.app".into()))
    );
    // A shorthand for a web search is listed with it, and starting it shows the search.
    let rows = shown(&mut s, "go", &["Grapher"]);
    assert_eq!(titles(&rows), ["Grapher", "Search Google"]);
    assert_eq!(rows[1].1, ["goo ␣", "g ␣"]);
}
