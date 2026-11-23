use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use dupin_core::clipboard::ClipboardProvider;
use dupin_core::snippets::{SnippetCatalog, SnippetProvider};
use dupin_core::{
    CommandId, CommandRegistry, CommandSpec, Config, History, Provider, QueryEligibility, QueryPresentation,
    Session,
};
use dupin_mac::{
    AppsProvider, CalendarProvider, DictionaryProvider, FolderProvider, RecentProvider, SpotlightProvider,
    Target,
};

pub const APPS: &str = "apps.search";
pub const FILES: &str = "files.names";
pub const CONTENTS: &str = "files.contents";
pub const DICTIONARY: &str = "dictionary.lookup";
pub const CALENDAR: &str = "calendar.events";
pub const CLIPBOARD: &str = "clipboard.history";
pub const SNIPPETS: &str = "snippets.search";
pub const SCRIPTS: &str = "scripts.search";
const BROWSE: &str = "files.browse";
const SETTINGS: &str = "settings.search";
const CALCULATOR: &str = "calculator.evaluate";
const WEB_KEYWORDS: &str = "web.keywords";
const WEB_FALLBACKS: &str = "web.fallbacks";
const WINDOWS: &str = "windows.arrange";
const MAX_FILE_CANDIDATES: usize = 10_000;

pub struct Registration {
    pub session: Session,
    pub providers: HashMap<CommandId, Arc<dyn Provider>>,
    pub notice: Option<String>,
    pub web: Arc<RwLock<crate::web::WebCatalog>>,
    pub snippets: Arc<RwLock<SnippetCatalog>>,
    pub scripts: Arc<RwLock<crate::scripts::catalog::Catalog>>,
    pub file_demotions: Arc<RwLock<Vec<String>>>,
}

pub struct ConfiguredCommands {
    pub registry: CommandRegistry,
    pub notice: Option<String>,
    pub web: crate::web::WebCatalog,
    pub snippets: SnippetCatalog,
}

fn parsed_file_query(
    command: &CommandId,
    raw: &str,
) -> Option<Result<dupin_core::file_query::FileQuery, dupin_core::file_query::FileQueryError>> {
    matches!(command.as_str(), FILES | CONTENTS).then(|| dupin_core::file_query::FileQuery::parse(raw))
}

pub fn query_qualifier_ranges(command: &CommandId, raw: &str) -> Vec<std::ops::Range<usize>> {
    parsed_file_query(command, raw)
        .and_then(Result::ok)
        .map(|query| query.qualifier_ranges)
        .unwrap_or_default()
}

pub fn query_problem(command: &CommandId, raw: &str) -> Option<&'static str> {
    match parsed_file_query(command, raw)? {
        Err(error) => Some(error.message()),
        Ok(query) if command.as_str() == CONTENTS && query.terms.is_empty() && !raw.trim().is_empty() => {
            Some("Type a word to search file contents.")
        }
        _ => None,
    }
}

/// Register `keywords` for `command`, describing the ones that couldn't be, under `setting`.
fn add_keywords(
    registry: &mut CommandRegistry,
    command: &CommandId,
    keywords: &[String],
    setting: &str,
) -> Option<String> {
    let rejected: Vec<_> = keywords
        .iter()
        .filter_map(|keyword| registry.add_keyword(command, keyword).err())
        .map(|error| error.to_string())
        .collect();
    (!rejected.is_empty()).then(|| {
        format!(
            "Some keywords were not registered. Review {setting} in config.toml: {}",
            rejected.join("; "),
        )
    })
}

fn id(value: &str) -> CommandId {
    CommandId::new(value).expect("built-in command IDs are valid")
}

fn recent_id() -> CommandId {
    id("recent.items")
}

fn presentation(
    placeholder: &str,
    chip: Option<&str>,
    empty_hint: Option<&str>,
    no_results_hint: Option<&str>,
    show_searching: bool,
    backdrop: f32,
) -> QueryPresentation {
    QueryPresentation {
        placeholder: placeholder.into(),
        chip: chip.map(str::to_owned),
        empty_hint: empty_hint.map(str::to_owned),
        no_results_hint: no_results_hint.map(str::to_owned),
        show_searching,
        backdrop,
    }
}

pub fn configured(config: &Config) -> Result<ConfiguredCommands, dupin_core::CommandError> {
    let file_limit = config.file_limit.min(MAX_FILE_CANDIDATES);
    let apps_id = id(APPS);
    let files_id = id(FILES);
    let contents_id = id(CONTENTS);
    let dictionary_id = id(DICTIONARY);
    let snippets_id = id(SNIPPETS);
    let mut registry = CommandRegistry::new(apps_id.clone(), Some(files_id.clone()));
    for command in [
        CommandSpec::new(
            apps_id.clone(),
            50,
            presentation("Search apps", None, None, Some("No matches"), false, 0.0),
        ),
        CommandSpec::new(
            files_id.clone(),
            file_limit,
            presentation(
                "Search files",
                Some("Files"),
                Some("Type to search files with Spotlight  ·  ⌫ back to apps"),
                Some("No matches"),
                true,
                1.0,
            ),
        )
        .with_candidate_limit(file_limit.saturating_mul(4).min(MAX_FILE_CANDIDATES)),
        CommandSpec::new(
            contents_id.clone(),
            file_limit,
            presentation(
                "Search inside files",
                Some("In files"),
                Some("Type to search inside files with Spotlight  ·  ⌫ back to apps"),
                Some("No matches"),
                true,
                2.0,
            ),
        ),
        CommandSpec::new(
            id(BROWSE),
            file_limit,
            presentation("Filter folder", None, None, Some("No matches"), false, 1.0),
        )
        .with_query_eligibility(QueryEligibility::Any)
        .with_candidate_limit(MAX_FILE_CANDIDATES),
        CommandSpec::new(
            dictionary_id.clone(),
            1,
            presentation(
                "Look up a word",
                Some("Define"),
                Some("Type a word  ·  ⌫ back to apps"),
                Some("No definition found"),
                true,
                3.0,
            ),
        ),
        CommandSpec::new(
            snippets_id.clone(),
            config.snippets.len(),
            presentation(
                "Search snippets",
                Some("Snippets"),
                Some("No snippets yet: add them to config.toml  ·  ⌫ back to apps"),
                Some("No matching snippets"),
                false,
                0.0,
            ),
        )
        .with_query_eligibility(QueryEligibility::Any),
    ] {
        registry.register(command)?;
    }
    registry.set_destination_search(files_id)?;

    registry.register(CommandSpec::new(
        id(SETTINGS),
        10,
        presentation("Search settings", None, None, None, false, 0.0),
    ))?;
    registry.register(CommandSpec::new(
        id(CALCULATOR),
        usize::from(config.calculator),
        presentation("Calculate", None, None, None, false, 0.0),
    ))?;
    registry.register(CommandSpec::new(
        id(WEB_KEYWORDS),
        1,
        presentation("Web search", None, None, None, false, 0.0),
    ))?;
    registry.register(
        CommandSpec::new(
            id(WEB_FALLBACKS),
            20,
            presentation("Web search", None, None, None, false, 0.0),
        )
        .with_fallback(),
    )?;
    registry.register(CommandSpec::new(
        id(SCRIPTS),
        8,
        presentation("Search scripts", None, None, None, false, 0.0),
    ))?;
    let mut default_providers = vec![
        apps_id,
        id(SCRIPTS),
        id(SETTINGS),
        id(WEB_KEYWORDS),
        id(WEB_FALLBACKS),
    ];
    if config.calculator {
        default_providers.insert(0, id(CALCULATOR));
    }
    if config.show_recent {
        let recent = recent_id();
        registry.register(
            CommandSpec::new(
                recent.clone(),
                10,
                presentation("Recent items", None, None, None, false, 0.0),
            )
            .with_query_eligibility(QueryEligibility::EmptyOnly),
        )?;
        default_providers.push(recent);
    }
    if config.window_management {
        registry.register(CommandSpec::new(
            id(WINDOWS),
            crate::window_management::COMMANDS.len(),
            presentation("Arrange windows", None, None, None, false, 0.0),
        ))?;
        default_providers.push(id(WINDOWS));
    }
    registry.set_default_providers(default_providers, Some(id(WEB_KEYWORDS)))?;

    registry.set_browser(id(BROWSE))?;
    registry.add_keyword(&contents_id, "in")?;
    if config.clipboard.enabled {
        let clipboard = id(CLIPBOARD);
        let mut spec = CommandSpec::new(
            clipboard.clone(),
            config.clipboard.max_entries,
            presentation(
                "Search clipboard history",
                Some("Clipboard"),
                Some("Nothing copied yet  ·  ⌫ back to apps"),
                Some("No matches"),
                false,
                4.0,
            ),
        )
        .with_query_eligibility(QueryEligibility::Any);
        spec.private_results = true;
        registry.register(spec)?;
        registry.add_keyword(&clipboard, "clip")?;
    }
    if config.calendar.enabled {
        let calendar = id(CALENDAR);
        let mut spec = CommandSpec::new(
            calendar.clone(),
            30,
            presentation(
                "Search calendar events",
                Some("Calendar"),
                Some("No upcoming events"),
                Some("No matching events"),
                true,
                5.0,
            ),
        )
        .with_query_eligibility(QueryEligibility::Any);
        spec.private_results = true;
        registry.register(spec)?;
        registry.add_keyword(&calendar, "cal")?;
    }
    let dictionary_notice = add_keywords(
        &mut registry,
        &dictionary_id,
        &config.dictionary_keywords,
        "dictionary_keywords",
    );
    let snippet_keyword_notice = add_keywords(
        &mut registry,
        &snippets_id,
        &config.snippet_keywords,
        "snippet_keywords",
    );

    let (web, web_notice) =
        crate::web::WebCatalog::compile(config, |keyword| registry.keyword(keyword).is_some());
    let (snippets, snippets_notice) = SnippetCatalog::compile(config);
    let notices: Vec<_> = [
        dictionary_notice,
        snippet_keyword_notice,
        web_notice,
        snippets_notice,
    ]
    .into_iter()
    .flatten()
    .collect();
    let notice = (!notices.is_empty()).then(|| notices.join("\n"));
    Ok(ConfiguredCommands {
        registry,
        notice,
        web,
        snippets,
    })
}

pub fn register(
    config: &Config,
    apps: Arc<AppsProvider>,
    history: History,
    calendar: CalendarProvider,
    clipboard: Arc<Mutex<dupin_core::clipboard::History>>,
    calculator: crate::calculator::Calculator,
) -> Result<Registration, dupin_core::CommandError> {
    let ConfiguredCommands {
        registry,
        notice,
        web,
        snippets,
    } = configured(config)?;
    let session = Session::new(history, registry)?;
    let web = Arc::new(RwLock::new(web));
    let snippets = Arc::new(RwLock::new(snippets));
    let apps_id = id(APPS);
    let files_id = id(FILES);
    let contents_id = id(CONTENTS);
    let dictionary_id = id(DICTIONARY);
    let file_demotions = Arc::new(RwLock::new(config.file_demotions.clone()));
    let scripts = Arc::new(RwLock::new(crate::scripts::catalog::Catalog::default()));
    let providers: HashMap<_, Arc<dyn Provider>> = [
        (apps_id, apps.clone() as Arc<dyn Provider>),
        (
            recent_id(),
            Arc::new(RecentProvider::new(apps, session.recent_history())) as Arc<dyn Provider>,
        ),
        (id(CALENDAR), Arc::new(calendar) as Arc<dyn Provider>),
        (
            id(SCRIPTS),
            Arc::new(crate::scripts::provider::ScriptsProvider(scripts.clone())) as Arc<dyn Provider>,
        ),
        (id(CALCULATOR), Arc::new(calculator) as Arc<dyn Provider>),
        (
            id(SETTINGS),
            Arc::new(dupin_mac::SettingsProvider::default()) as Arc<dyn Provider>,
        ),
        (
            id(WINDOWS),
            Arc::new(crate::window_management::WindowProvider) as Arc<dyn Provider>,
        ),
        (
            files_id,
            Arc::new(SpotlightProvider::with_demotions(
                Target::Names,
                file_demotions.clone(),
            )) as Arc<dyn Provider>,
        ),
        (
            contents_id,
            Arc::new(SpotlightProvider::new(Target::Contents)) as Arc<dyn Provider>,
        ),
        (
            dictionary_id,
            Arc::new(DictionaryProvider::default()) as Arc<dyn Provider>,
        ),
        (id(BROWSE), Arc::new(FolderProvider) as Arc<dyn Provider>),
        (
            id(SNIPPETS),
            Arc::new(SnippetProvider::new(snippets.clone())) as Arc<dyn Provider>,
        ),
        (
            id(CLIPBOARD),
            Arc::new(ClipboardProvider::new(
                clipboard,
                dupin_mac::clipboard::access_item,
            )) as Arc<dyn Provider>,
        ),
        (
            id(WEB_KEYWORDS),
            Arc::new(crate::web::WebProvider::new(web.clone(), false)) as Arc<dyn Provider>,
        ),
        (
            id(WEB_FALLBACKS),
            Arc::new(crate::web::WebProvider::new(web.clone(), true)) as Arc<dyn Provider>,
        ),
    ]
    .into();

    Ok(Registration {
        session,
        providers,
        notice,
        web,
        snippets,
        file_demotions,
        scripts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculator_precedes_apps_and_enter_copies_without_recording_usage() {
        use dupin_core::{Action, Delivery, Effect, History, Intent, Item, Request, Session, View};

        let registry = configured(&Config::default()).unwrap().registry;
        let mut session = Session::new(History::default(), registry).unwrap();
        let effects = session.set_query("12*3");
        let generation = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Search {
                    command, generation, ..
                } if command.as_str() == CALCULATOR => Some(*generation),
                _ => None,
            })
            .unwrap();
        let mut items = Vec::new();
        crate::calculator::Calculator::default()
            .search(&Request::detached("12*3", 1), &mut |batch| items.extend(batch))
            .unwrap();
        session.deliver_from(
            id(APPS),
            Delivery::Batch {
                generation,
                items: vec![Item::path("/fixture.app".into(), "Fixture".into(), Vec::new())],
            },
        );
        session.deliver_from(id(CALCULATOR), Delivery::Batch { generation, items });
        assert_eq!(
            session.selected_item().unwrap().view,
            View::Pair {
                source: "12*3".into(),
                result: "36".into()
            }
        );
        assert_eq!(session.items()[1].display_title(), "Fixture");
        let activate = session.intent(Intent::Open).unwrap();
        assert!(activate.contains(&Effect::Run(Action::Copy("36".into()))));
        assert!(!activate.contains(&Effect::SaveHistory));
    }

    #[test]
    fn disabling_calculator_refreshes_default_search_without_it() {
        use dupin_core::{Delivery, Effect, History, Session};

        let registry = configured(&Config::parse("file_limit = 20").unwrap())
            .unwrap()
            .registry;
        let mut session = Session::new(History::default(), registry).unwrap();
        let effects = session.set_query("12*3");
        let generation = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Search {
                    command, generation, ..
                } if command.as_str() == CALCULATOR => Some(*generation),
                _ => None,
            })
            .unwrap();

        let disabled = Config::parse("calculator = false").unwrap();
        let effects = session
            .set_commands(configured(&disabled).unwrap().registry)
            .unwrap();
        assert!(!effects.is_empty());
        assert!(effects.iter().all(
            |effect| !matches!(effect, Effect::Search { command, .. } if command.as_str() == CALCULATOR)
        ));
        assert!(!session.deliver_from(id(CALCULATOR), Delivery::Done { generation }));
    }

    #[test]
    fn window_commands_follow_the_live_opt_in_and_run_through_the_session() {
        use dupin_core::window_management::Layout;
        use dupin_core::{Action, Delivery, Effect, History, Intent, Request, Session};

        let mut config = Config::parse("").unwrap();
        assert!(!config.window_management);
        let mut session = Session::new(History::default(), configured(&config).unwrap().registry).unwrap();
        let windows = id(WINDOWS);
        let searches_windows =
            |effect: &Effect| matches!(effect, Effect::Search { command, .. } if command == &windows);
        assert!(!session.set_query("left half").iter().any(searches_windows));

        config = Config::parse("window_management = true").unwrap();
        session
            .set_commands(configured(&config).unwrap().registry)
            .unwrap();
        let generation = session
            .refresh()
            .into_iter()
            .find_map(|effect| match effect {
                Effect::Search {
                    command, generation, ..
                } if command == windows => Some(generation),
                _ => None,
            })
            .unwrap();
        let mut items = Vec::new();
        crate::window_management::WindowProvider
            .search(&Request::detached("left half", 10), &mut |batch| {
                items.extend(batch)
            })
            .unwrap();
        session.deliver_from(windows.clone(), Delivery::Batch { generation, items });
        let effects = session.intent(Intent::Open).unwrap();
        assert!(effects.contains(&Effect::Run(Action::ArrangeWindow(Layout::LeftHalf))));
        assert!(effects.contains(&Effect::Hide));

        session
            .set_commands(configured(&Config::default()).unwrap().registry)
            .unwrap();
        assert!(!session.set_query("left half").iter().any(searches_windows));
    }

    #[test]
    fn recent_is_the_only_blank_search_provider_and_can_be_disabled() {
        use dupin_core::{Effect, Session};

        let mut config = Config::default();
        let mut session = Session::new(History::default(), configured(&config).unwrap().registry).unwrap();
        assert!(matches!(
            session.refresh().as_slice(),
            [Effect::Search { command, query, .. }] if *command == recent_id() && query.is_empty()
        ));

        config.show_recent = false;
        let effects = session
            .set_commands(configured(&config).unwrap().registry)
            .unwrap();
        assert_eq!(effects, [Effect::CancelSearch]);
        assert!(session.items().is_empty());
    }

    #[test]
    fn calendar_queries_use_an_opt_in_explicit_route_with_upcoming_results() {
        use dupin_core::{Action, Delivery, Effect, Intent, Request, Session};

        let base = configured(&Config::default()).unwrap();
        assert!(base.registry.keyword("cal").is_none());
        let enabled = Config::parse("[calendar]\nenabled = true").unwrap();
        let mut session = Session::new(History::default(), configured(&enabled).unwrap().registry).unwrap();
        let calendar = id(CALENDAR);
        let ordinary = session.set_query("cal");
        assert!(
            !ordinary
                .iter()
                .any(|effect| matches!(effect, Effect::Search { command, .. } if command == &calendar))
        );

        let effects = session.intent(Intent::Space).unwrap();
        let [
            Effect::ClearQuery,
            Effect::Search {
                command,
                generation,
                query,
                ..
            },
        ] = effects.as_slice()
        else {
            panic!("calendar mode should search immediately");
        };
        assert_eq!(command, &calendar);
        assert!(query.is_empty());
        let generation = *generation;
        let mut rows = Vec::new();
        CalendarProvider::fixtures()
            .search(&Request::detached(query, 30), &mut |batch| rows.extend(batch))
            .unwrap();
        session.deliver_from(
            calendar.clone(),
            Delivery::Batch {
                generation,
                items: rows,
            },
        );
        session.deliver_from(calendar.clone(), Delivery::Done { generation });
        assert_eq!(session.items().len(), 3);
        assert!(session.capture().is_err());
        session.intent(Intent::Up).unwrap();
        assert_eq!(session.selected_item().unwrap().display_title(), "Out of office");
        let effects = session.intent(Intent::Open).unwrap();
        assert!(effects.iter().any(
            |effect| matches!(effect, Effect::Run(Action::OpenUrl(url)) if url.starts_with("ical://ekevent/"))
        ));
        assert!(!effects.contains(&Effect::SaveHistory));
        assert!(!session.deliver_from(calendar, Delivery::Done { generation }));
    }

    #[test]
    fn web_results_use_the_registered_session_action_and_completion_paths() {
        use dupin_core::{Action, Delivery, Effect, History, Intent, Request, Session};

        let configured = configured(&Config::default()).unwrap();
        let web = crate::web::WebProvider::new(Arc::new(RwLock::new(configured.web)), false);
        let mut session = Session::new(History::default(), configured.registry).unwrap();
        let effects = session.set_query("g");
        let generation = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Search { generation, .. } => Some(*generation),
                _ => None,
            })
            .unwrap();
        let mut rows = Vec::new();
        web.search(&Request::detached("g", 1), &mut |batch| rows.extend(batch))
            .unwrap();
        session.deliver_from(
            id(WEB_KEYWORDS),
            Delivery::Batch {
                generation,
                items: rows,
            },
        );
        let completion = session.intent(Intent::Complete).unwrap();
        assert!(completion.contains(&Effect::ReplaceQuery("g ".into())));
        assert_eq!(session.query(), "g ");

        let effects = session.set_query("gh dupin & Rust");
        let generation = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Search { generation, .. } => Some(*generation),
                _ => None,
            })
            .unwrap();
        let mut rows = Vec::new();
        web.search(&Request::detached("gh dupin & Rust", 1), &mut |batch| {
            rows.extend(batch)
        })
        .unwrap();
        session.deliver_from(
            id(WEB_KEYWORDS),
            Delivery::Batch {
                generation,
                items: rows,
            },
        );
        let activation = session.intent(Intent::Open).unwrap();
        assert!(activation.contains(&Effect::Run(Action::OpenUrl(
            "https://github.com/search?q=dupin%20%26%20Rust".into(),
        ))));
        assert!(!activation.contains(&Effect::SaveHistory));
    }

    #[test]
    fn clip_is_reserved_only_while_clipboard_history_is_enabled() {
        use dupin_core::{Delivery, Effect, Session};

        for enabled in [false, true] {
            let mut config = Config::default();
            config.clipboard.enabled = enabled;
            config.web_searches.insert(
                "clip".into(),
                dupin_core::config::WebSearch {
                    name: "Clips".into(),
                    url: "https://clips.example/?q={query}".into(),
                },
            );
            let commands = configured(&config).unwrap();
            let clipboard = id(CLIPBOARD);
            assert_eq!(commands.registry.keyword("clip"), enabled.then_some(&clipboard));
            assert_eq!(commands.notice.is_some(), enabled);

            let mut session = Session::new(History::default(), commands.registry).unwrap();
            let effects = session.set_query("clip term");
            if enabled {
                assert!(session.command().private_results);
                let [
                    Effect::ReplaceQuery(_),
                    Effect::Search {
                        command,
                        generation,
                        query,
                        ..
                    },
                ] = effects.as_slice()
                else {
                    panic!("clip should enter clipboard history: {effects:?}");
                };
                assert_eq!(command, &clipboard);
                assert_eq!(query, "term");

                config.clipboard.enabled = false;
                let effects = session
                    .set_commands(configured(&config).unwrap().registry)
                    .unwrap();
                assert!(effects.contains(&Effect::ClearQuery));
                assert_eq!(session.command_id(), &id(APPS));
                assert!(!session.deliver_from(
                    clipboard,
                    Delivery::Done {
                        generation: *generation
                    }
                ));
            } else {
                assert_eq!(session.command_id(), &id(APPS));
                assert!(session.provider_spec(&clipboard).is_none());
                assert!(effects.iter().any(|effect| matches!(
                    effect,
                    Effect::Search { command, query, .. } if *command == id(WEB_KEYWORDS) && query == "clip term"
                )));
            }
        }
    }

    #[test]
    fn invalid_dictionary_keywords_are_skipped_with_a_notice() {
        let config = Config {
            dictionary_keywords: vec!["in".into(), "".into(), "look up".into()],
            ..Config::default()
        };

        let configured = configured(&config).unwrap();
        let dictionary = id(DICTIONARY);
        assert_eq!(configured.registry.keyword("in"), Some(&id(CONTENTS)));
        assert_eq!(configured.registry.keyword("look up"), Some(&dictionary));
        let notice = configured.notice.unwrap();
        assert!(notice.contains("dictionary_keywords"));
        assert!(!notice.contains("snippet_keywords"));
        assert!(notice.contains("in"));
        assert!(notice.contains("must not be empty"));
    }

    #[test]
    fn snippet_keywords_route_to_snippets_and_reserve_web_keywords() {
        use dupin_core::config::{Snippet, WebSearch};
        use dupin_core::{Effect, Session};

        let config = Config {
            snippet_keywords: vec!["s".into(), "in".into()],
            snippets: [(
                "sig".into(),
                Snippet {
                    name: None,
                    text: "Best wishes".into(),
                },
            )]
            .into(),
            web_searches: [(
                "s".into(),
                WebSearch {
                    name: "Search".into(),
                    url: "https://example.com/?q={query}".into(),
                },
            )]
            .into(),
            ..Config::default()
        };
        let configured = configured(&config).unwrap();
        assert_eq!(configured.registry.keyword("s"), Some(&id(SNIPPETS)));
        assert_eq!(configured.registry.keyword("in"), Some(&id(CONTENTS)));
        assert_eq!(configured.registry.keyword("snip"), None);
        let notice = configured.notice.unwrap();
        assert!(notice.contains("snippet_keywords"));
        assert!(notice.contains("web searches"));
        assert_ne!(configured.snippets, SnippetCatalog::default());
        let mut session = Session::new(History::default(), configured.registry).unwrap();
        assert!(matches!(
            session.set_query("s ").as_slice(),
            [Effect::ReplaceQuery(text), Effect::Search { command, query, .. }]
                if text.is_empty() && *command == id(SNIPPETS) && query.is_empty()
        ));
    }

    #[test]
    fn empty_snippet_search_lists_the_whole_catalogue() {
        use dupin_core::config::Snippet;
        use dupin_core::{Delivery, Effect, Request};

        let config = Config {
            snippets: (0..64)
                .map(|i| {
                    (
                        format!("snippet-{i:02}"),
                        Snippet {
                            name: None,
                            text: format!("Example {i}"),
                        },
                    )
                })
                .collect(),
            ..Config::default()
        };
        let configured = configured(&config).unwrap();
        let provider = SnippetProvider::new(Arc::new(RwLock::new(configured.snippets)));
        let mut session = Session::new(History::default(), configured.registry).unwrap();
        let effects = session.set_query("snip ");
        let [
            Effect::ReplaceQuery(_),
            Effect::Search {
                command,
                generation,
                query,
                ..
            },
        ] = effects.as_slice()
        else {
            panic!("expected a search in snippets mode, got {effects:?}");
        };
        let limit = session.provider_spec(command).unwrap().candidate_limit();
        provider
            .search(&Request::detached(query, limit), &mut |items| {
                session.deliver_from(
                    command.clone(),
                    Delivery::Batch {
                        generation: *generation,
                        items,
                    },
                );
            })
            .unwrap();
        assert_eq!(session.items().len(), config.snippets.len());
    }

    #[test]
    fn qualifier_feedback_matches_only_the_file_commands() {
        let raw = "kind:pdf report";
        for command in [FILES, CONTENTS] {
            let ranges = query_qualifier_ranges(&id(command), raw);
            assert_eq!(ranges.len(), 1);
            assert_eq!(ranges[0], 0..8);
        }
        assert!(query_qualifier_ranges(&id(APPS), raw).is_empty());
        assert!(query_problem(&id(APPS), "kind:").is_none());
        assert!(query_problem(&id(FILES), "kind:").is_some());
        assert!(query_problem(&id(FILES), "kind:pdf").is_none());
        assert_eq!(
            query_problem(&id(CONTENTS), "kind:pdf"),
            Some("Type a word to search file contents.")
        );
        assert!(query_problem(&id(CONTENTS), raw).is_none());
        assert!(query_problem(&id(CONTENTS), "").is_none());
    }

    #[test]
    fn only_name_search_oversamples_with_a_bounded_candidate_budget() {
        for requested in [0, 2, 4000, usize::MAX] {
            let config = Config {
                file_limit: requested,
                ..Config::default()
            };
            let configured = configured(&config).unwrap();
            let session =
                dupin_core::Session::new(dupin_core::History::default(), configured.registry).unwrap();
            let names = session.provider_spec(&id(FILES)).unwrap();
            let contents = session.provider_spec(&id(CONTENTS)).unwrap();
            let shown = requested.min(MAX_FILE_CANDIDATES);
            assert_eq!(names.result_limit, shown);
            assert_eq!(
                names.candidate_limit(),
                shown.saturating_mul(4).min(MAX_FILE_CANDIDATES)
            );
            assert_eq!(contents.result_limit, shown);
            assert_eq!(contents.candidate_limit(), shown);
            let apps = session.provider_spec(&id(APPS)).unwrap();
            assert_eq!(apps.candidate_limit(), apps.result_limit);
        }
    }
}
