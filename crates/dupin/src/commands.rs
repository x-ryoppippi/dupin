use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use dupin_core::{CommandId, CommandRegistry, CommandSpec, Config, Provider, QueryPresentation};
use dupin_mac::{AppsProvider, DictionaryProvider, SpotlightProvider, Target};

pub const APPS: &str = "apps.search";
pub const FILES: &str = "files.names";
pub const CONTENTS: &str = "files.contents";
pub const DICTIONARY: &str = "dictionary.lookup";
const WEB_KEYWORDS: &str = "web.keywords";
const WEB_FALLBACKS: &str = "web.fallbacks";

pub struct Registration {
    pub registry: CommandRegistry,
    pub providers: HashMap<CommandId, Arc<dyn Provider>>,
    pub notice: Option<String>,
    pub web: Arc<RwLock<crate::web::WebCatalog>>,
}

pub struct ConfiguredCommands {
    pub registry: CommandRegistry,
    pub notice: Option<String>,
    pub web: crate::web::WebCatalog,
}

fn id(value: &str) -> CommandId {
    CommandId::new(value).expect("built-in command IDs are valid")
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
    let apps_id = id(APPS);
    let files_id = id(FILES);
    let contents_id = id(CONTENTS);
    let dictionary_id = id(DICTIONARY);
    let mut registry = CommandRegistry::new(apps_id.clone(), Some(files_id.clone()));
    for command in [
        CommandSpec::new(
            apps_id.clone(),
            50,
            presentation("Search apps", None, None, Some("No matches"), false, 0.0),
        ),
        CommandSpec::new(
            files_id.clone(),
            config.file_limit,
            presentation(
                "Search files",
                Some("Files"),
                Some("Type to search files with Spotlight  ·  ⌫ back to apps"),
                Some("No matches"),
                true,
                1.0,
            ),
        ),
        CommandSpec::new(
            contents_id.clone(),
            config.file_limit,
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
    ] {
        registry.register(command)?;
    }

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
    registry.set_default_providers(
        vec![apps_id, id(WEB_KEYWORDS), id(WEB_FALLBACKS)],
        Some(id(WEB_KEYWORDS)),
    )?;

    registry.add_keyword(&contents_id, "in")?;
    let mut rejected = Vec::new();
    for keyword in &config.dictionary_keywords {
        if let Err(error) = registry.add_keyword(&dictionary_id, keyword) {
            rejected.push(error.to_string());
        }
    }
    let dictionary_notice = (!rejected.is_empty()).then(|| {
        format!(
            "Some dictionary keywords were not registered. Review dictionary_keywords in config.toml: {}",
            rejected.join("; "),
        )
    });

    let (web, web_notice) = crate::web::WebCatalog::compile(config);
    let notices: Vec<_> = [dictionary_notice, web_notice].into_iter().flatten().collect();
    let notice = (!notices.is_empty()).then(|| notices.join("\n"));
    Ok(ConfiguredCommands {
        registry,
        notice,
        web,
    })
}

pub fn register(config: &Config, apps: Arc<AppsProvider>) -> Result<Registration, dupin_core::CommandError> {
    let ConfiguredCommands {
        registry,
        notice,
        web,
    } = configured(config)?;
    let web = Arc::new(RwLock::new(web));
    let apps_id = id(APPS);
    let files_id = id(FILES);
    let contents_id = id(CONTENTS);
    let dictionary_id = id(DICTIONARY);
    let providers: HashMap<_, Arc<dyn Provider>> = [
        (apps_id, apps as Arc<dyn Provider>),
        (
            files_id,
            Arc::new(SpotlightProvider::new(Target::Names)) as Arc<dyn Provider>,
        ),
        (
            contents_id,
            Arc::new(SpotlightProvider::new(Target::Contents)) as Arc<dyn Provider>,
        ),
        (
            dictionary_id,
            Arc::new(DictionaryProvider::default()) as Arc<dyn Provider>,
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
        registry,
        providers,
        notice,
        web,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(notice.contains("in"));
        assert!(notice.contains("must not be empty"));
    }
}
