use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use dupin_core::snippets::SnippetCatalog;
use dupin_core::{
    CommandError, CommandId, CommandRegistry, Config, History, Provider, QueryPresentation, Session,
};
use dupin_mac::{AppsProvider, CalendarProvider, system_commands};

use crate::features::{MANIFEST, apps::APPS, files::FILES};

pub struct Registration {
    pub session: Session,
    pub providers: HashMap<CommandId, Arc<dyn Provider>>,
    pub notice: Option<String>,
    pub web: Arc<RwLock<crate::web::WebSearches>>,
    pub snippets: Arc<RwLock<SnippetCatalog>>,
    pub scripts: Arc<RwLock<crate::scripts::catalog::Catalog>>,
    pub file_demotions: Arc<RwLock<Vec<String>>>,
    pub system_provider: Arc<system_commands::SystemProvider>,
}

pub struct ConfiguredCommands {
    pub registry: CommandRegistry,
    pub notice: Option<String>,
    pub web: crate::web::WebCatalog,
    pub snippets: SnippetCatalog,
}

pub(super) fn id(value: &str) -> CommandId {
    CommandId::new(value).expect("built-in command IDs are valid")
}

pub(super) fn presentation(
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

pub(super) struct Configure {
    pub registry: CommandRegistry,
    pub defaults: Vec<CommandId>,
    pub pinned: Option<CommandId>,
    pub notices: Vec<String>,
    pub web: crate::web::WebCatalog,
    pub snippets: SnippetCatalog,
}

impl Configure {
    pub fn add_keywords(&mut self, command: &CommandId, keywords: &[String], setting: &str) {
        let rejected: Vec<_> = keywords
            .iter()
            .filter_map(|keyword| self.registry.add_keyword(command, keyword).err())
            .map(|error| error.to_string())
            .collect();
        if !rejected.is_empty() {
            self.notices.push(format!(
                "Some keywords were not registered. Review {setting} in config.toml: {}",
                rejected.join("; "),
            ));
        }
    }
}

pub fn configured(config: &Config) -> Result<ConfiguredCommands, CommandError> {
    let mut commands = Configure {
        registry: CommandRegistry::new(id(APPS), Some(id(FILES))),
        defaults: Vec::new(),
        pinned: None,
        notices: Vec::new(),
        web: crate::web::WebCatalog::default(),
        snippets: SnippetCatalog::default(),
    };
    for feature in MANIFEST {
        (feature.configure)(config, &mut commands)?;
    }
    commands
        .registry
        .set_default_providers(std::mem::take(&mut commands.defaults), commands.pinned.take())?;
    for feature in MANIFEST {
        (feature.prepare)(config, &mut commands);
    }
    Ok(ConfiguredCommands {
        registry: commands.registry,
        notice: (!commands.notices.is_empty()).then(|| commands.notices.join("\n")),
        web: commands.web,
        snippets: commands.snippets,
    })
}

pub(super) struct Providers<'a> {
    pub registration: &'a mut Registration,
    pub apps: Arc<AppsProvider>,
    pub calendar: Arc<CalendarProvider>,
    pub clipboard: Arc<Mutex<dupin_core::clipboard::History>>,
    pub calculator: Arc<crate::calculator::Calculator>,
}

impl Providers<'_> {
    pub fn add(&mut self, command: &str, provider: Arc<dyn Provider>) {
        self.registration.providers.insert(id(command), provider);
    }
}

pub fn register(
    config: &Config,
    apps: Arc<AppsProvider>,
    history: History,
    calendar: CalendarProvider,
    clipboard: Arc<Mutex<dupin_core::clipboard::History>>,
    calculator: crate::calculator::Calculator,
) -> Result<Registration, CommandError> {
    let ConfiguredCommands {
        registry,
        notice,
        web,
        snippets,
    } = configured(config)?;
    let session = Session::new(history, registry)?;
    let web = Arc::new(RwLock::new(crate::web::WebSearches::new(
        web,
        Vec::new(),
        std::collections::HashSet::new(),
    )));
    let snippets = Arc::new(RwLock::new(snippets));
    let file_demotions = Arc::new(RwLock::new(config.file_demotions.clone()));
    let scripts = Arc::new(RwLock::new(crate::scripts::catalog::Catalog::default()));
    let system_provider = Arc::new(system_commands::SystemProvider::new(config.system_automation));
    let mut registration = Registration {
        session,
        providers: HashMap::new(),
        notice,
        web,
        snippets,
        file_demotions,
        scripts,
        system_provider,
    };
    let mut providers = Providers {
        registration: &mut registration,
        apps,
        calendar: Arc::new(calendar),
        clipboard,
        calculator: Arc::new(calculator),
    };
    for feature in MANIFEST {
        (feature.start)(&mut providers);
    }
    Ok(registration)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::{
        calculator::CALCULATOR,
        calendar::CALENDAR,
        clipboard::CLIPBOARD,
        dictionary::DICTIONARY,
        extensions::add_extensions,
        files::{CONTENTS, MAX_FILE_CANDIDATES, query_problem, query_qualifier_ranges},
        recent::RECENT,
        snippets::SNIPPETS,
        system::SYSTEM,
        web::WEB_KEYWORDS,
        windows::WINDOWS,
    };
    use dupin_core::snippets::SnippetProvider;

    #[test]
    fn manifest_preserves_command_ids_provider_order_and_keywords() {
        use dupin_core::{Effect, config::WebSearch};

        let mut config = Config {
            calculator: true,
            show_recent: true,
            window_management: true,
            dictionary_keywords: ["in", "clip", "cal", "define", "shared"]
                .map(str::to_owned)
                .into(),
            snippet_keywords: ["shared", "snip"].map(str::to_owned).into(),
            web_searches: ["in", "clip", "cal", "define", "shared", "snip", "web"]
                .into_iter()
                .map(|keyword| {
                    (
                        keyword.into(),
                        WebSearch {
                            name: "Example".into(),
                            url: "https://example.com/?q={query}".into(),
                        },
                    )
                })
                .collect(),
            web_fallbacks: vec!["web".into()],
            ..Config::default()
        };
        config.calendar.enabled = true;
        config.clipboard.enabled = true;
        let ConfiguredCommands { registry, web, .. } = configured(&config).unwrap();

        let mut ids = registry.ids();
        ids.sort();
        assert_eq!(
            ids,
            [
                "apps.search",
                "calculator.evaluate",
                "calendar.events",
                "clipboard.history",
                "dictionary.lookup",
                "files.browse",
                "files.contents",
                "files.names",
                "recent.items",
                "scripts.search",
                "settings.search",
                "snippets.search",
                "system.commands",
                "web.fallbacks",
                "web.keywords",
                "windows.arrange",
            ]
        );
        for (keyword, expected) in [
            ("in", Some("files.contents")),
            ("clip", Some("clipboard.history")),
            ("cal", Some("calendar.events")),
            ("define", Some("dictionary.lookup")),
            ("shared", Some("dictionary.lookup")),
            ("snip", Some("snippets.search")),
            ("web", None),
        ] {
            assert_eq!(registry.keyword(keyword).map(CommandId::as_str), expected);
        }
        assert_eq!(
            web.search_urls("example")
                .map(|(keyword, _, _)| keyword)
                .collect::<Vec<_>>(),
            ["web"]
        );

        let mut session = Session::new(History::default(), registry).unwrap();
        assert!(matches!(
            session.refresh().as_slice(),
            [Effect::Search { command, .. }] if command.as_str() == "recent.items"
        ));
        let effects = session.set_query("manifest");
        let providers: Vec<_> = effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Search { command, .. } => Some(command.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            providers,
            [
                "calculator.evaluate",
                "apps.search",
                "system.commands",
                "scripts.search",
                "settings.search",
                "web.keywords",
                "web.fallbacks",
                "windows.arrange",
            ]
        );
    }

    #[test]
    fn normal_search_dispatches_each_command_through_its_expected_activation() {
        use dupin_core::{Action, Delivery, Effect, History, Intent, Request, Session, SystemCommand};

        let config = Config {
            system_automation: true,
            ..Config::default()
        };
        let provider = system_commands::SystemProvider::new(config.system_automation);
        for (command, title, requires_confirmation) in [
            (SystemCommand::Sleep, "Sleep", false),
            (SystemCommand::LockScreen, "Lock Screen", false),
            (SystemCommand::SleepDisplays, "Sleep Displays", false),
            (SystemCommand::Restart, "Restart", true),
            (SystemCommand::ShutDown, "Shut Down", true),
            (SystemCommand::LogOut, "Log Out", true),
            (SystemCommand::EmptyTrash, "Empty Trash", true),
            (SystemCommand::ToggleDarkMode, "Toggle Dark Mode", false),
        ] {
            let configured = crate::commands::configured(&config).unwrap();
            let origin = CommandId::new(SYSTEM).unwrap();
            let mut session = Session::new(History::default(), configured.registry).unwrap();
            let generation = session
                .set_query(title)
                .into_iter()
                .find_map(|effect| match effect {
                    Effect::Search {
                        command, generation, ..
                    } if command == origin => Some(generation),
                    _ => None,
                })
                .unwrap();
            provider
                .search(
                    &Request::detached(title, system_commands::COMMANDS.len()),
                    &mut |items| {
                        session.deliver_from(origin.clone(), Delivery::Batch { generation, items });
                    },
                )
                .unwrap();
            assert_eq!(session.selected_item().unwrap().display_title(), title);
            let key = session.result_key(0);
            let opened = session.intent(Intent::Open).unwrap();
            let effects = if requires_confirmation {
                assert_eq!(
                    opened,
                    [Effect::ShowActions {
                        id: key.clone(),
                        generation,
                        confirmation: Some("run".into()),
                    }]
                );
                assert!(session.run_key_action(&key, "run").is_empty());
                session.run_confirmed_key_action(&key, "run")
            } else {
                assert!(session.run_confirmed_key_action(&key, "run").is_empty());
                opened
            };
            let [Effect::Run(Action::System(picked)), ..] = effects.as_slice() else {
                panic!("expected a system command, got {effects:?}");
            };
            system_commands::execute(*picked, true, |picked| {
                assert_eq!(picked, command);
                Ok(())
            })
            .unwrap();
            assert!(!effects.contains(&Effect::SaveHistory));
        }
    }

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
            [Effect::Search { command, query, .. }] if *command == id(RECENT) && query.is_empty()
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
        CalendarProvider::fixtures()
            .search_batches(&Request::detached(query, 30), &mut |batch| {
                session.deliver_from(calendar.clone(), Delivery::Presented { generation, batch });
            })
            .unwrap();
        session.deliver_from(calendar.clone(), Delivery::Done { generation });
        assert_eq!(session.items().len(), 4);
        assert_eq!(session.section_start(0), Some("Today"));
        assert_eq!(session.section_start(2), Some("Tomorrow"));
        assert!(session.capture().is_err());
        session.intent(Intent::Up).unwrap();
        assert_eq!(session.selected_item().unwrap().display_title(), "Dentist");
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
        let web = crate::web::WebProvider::new(
            Arc::new(RwLock::new(crate::web::WebSearches::new(
                configured.web,
                Vec::new(),
                std::collections::HashSet::new(),
            ))),
            false,
        );
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
    fn extensions_get_keyword_routes_without_taking_existing_keywords() {
        use crate::extensions::tests::extension;

        let config = Config {
            dictionary_keywords: vec!["define".into()],
            ..Config::default()
        };
        let mut configured = configured(&config).unwrap();
        let extensions: Vec<_> = [
            ("Word Count", "wc"),
            ("Dictionary Clash", "define"),
            ("Contents Clash", "in"),
            ("Web Clash", "gh"),
            ("Snippet Clash", "snip"),
            ("Second Count", "wc"),
        ]
        .into_iter()
        .map(|(name, keyword)| Arc::new(extension(name, keyword)))
        .collect();
        let added = add_extensions(&mut configured, &extensions);

        let words = id("extension.word-count");
        assert_eq!(added.registered.len(), 1);
        assert_eq!(added.registered[0].0, words);
        assert_eq!(configured.registry.keyword("wc"), Some(&words));
        assert_eq!(configured.registry.keyword("define"), Some(&id(DICTIONARY)));
        assert_eq!(configured.registry.keyword("in"), Some(&id(CONTENTS)));
        assert_eq!(configured.registry.keyword("snip"), Some(&id(SNIPPETS)));
        assert!(configured.web.has_keyword("gh"));
        assert_eq!(added.problems.len(), 5, "{:?}", added.problems);
        for (problem, folder) in added.problems.iter().zip([
            "dictionary-clash-folder",
            "contents-clash-folder",
            "web-clash-folder",
            "snippet-clash-folder",
            "second-count-folder",
        ]) {
            assert!(
                problem.starts_with(folder) && problem.contains("already used"),
                "{problem}"
            );
        }

        let mut session =
            dupin_core::Session::new(dupin_core::History::default(), configured.registry).unwrap();
        session.set_query("wc two words");
        assert_eq!(session.command_id(), &words);
        assert_eq!(session.query(), "two words");
    }

    #[test]
    fn catalogues_that_ask_for_it_join_the_main_search_after_apps_in_folder_order() {
        use crate::extensions::Role;
        use crate::extensions::tests::extension;
        use dupin_core::Effect;

        let mut configured = configured(&Config::default()).unwrap();
        let extensions: Vec<_> = [
            ("Bookmarks", "bm", true),
            ("Words", "wc", false),
            ("Clips", "cl", true),
        ]
        .into_iter()
        .map(|(name, keyword, default_search)| {
            let mut extension = extension(name, keyword);
            extension.default_search = default_search;
            Arc::new(extension)
        })
        .collect();
        let added = add_extensions(&mut configured, &extensions);
        let roles: Vec<_> = added
            .registered
            .iter()
            .map(|(command, _, role)| (command.as_str(), *role))
            .collect();
        assert_eq!(
            roles,
            [
                ("extension.bookmarks", Role::Keyword),
                ("extension-search.bookmarks", Role::DefaultSearch),
                ("extension.words", Role::Keyword),
                ("extension.clips", Role::Keyword),
                ("extension-search.clips", Role::DefaultSearch),
            ]
        );
        let mut session = Session::new(History::default(), configured.registry).unwrap();
        let searched: Vec<_> = session
            .set_query("anything")
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::Search { command, .. } => Some(command.as_str().to_owned()),
                _ => None,
            })
            .collect();
        let apps = searched
            .iter()
            .position(|command| command == "apps.search")
            .unwrap();
        assert_eq!(
            searched[apps + 1..apps + 3],
            ["extension-search.bookmarks", "extension-search.clips"]
        );
        // The keyword command lists its catalogue before anything is typed.
        let effects = session.set_query("bm ");
        assert!(
            effects.iter().any(|effect| matches!(
                effect,
                Effect::Search { command, query, .. } if command.as_str() == "extension.bookmarks" && query.is_empty()
            )),
            "{effects:?}"
        );
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
