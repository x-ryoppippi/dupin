use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use dupin_core::{CommandId, CommandRegistry, CommandSpec, Config, Provider, QueryPresentation};
use dupin_mac::{AppsProvider, DictionaryProvider, SpotlightProvider, Target};

pub const APPS: &str = "apps.search";
pub const FILES: &str = "files.names";
pub const CONTENTS: &str = "files.contents";
pub const DICTIONARY: &str = "dictionary.lookup";
const MAX_FILE_CANDIDATES: usize = 10_000;

pub struct Registration {
    pub registry: CommandRegistry,
    pub providers: HashMap<CommandId, Arc<dyn Provider>>,
    pub notice: Option<String>,
    pub file_demotions: Arc<RwLock<Vec<String>>>,
}

pub struct ConfiguredCommands {
    pub registry: CommandRegistry,
    pub notice: Option<String>,
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
    let file_limit = config.file_limit.min(MAX_FILE_CANDIDATES);
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

    registry.add_keyword(&contents_id, "in")?;
    let mut rejected = Vec::new();
    for keyword in &config.dictionary_keywords {
        if let Err(error) = registry.add_keyword(&dictionary_id, keyword) {
            rejected.push(error.to_string());
        }
    }
    let notice = (!rejected.is_empty()).then(|| {
        format!(
            "Some dictionary keywords were not registered. Review dictionary_keywords in config.toml: {}",
            rejected.join("; "),
        )
    });

    Ok(ConfiguredCommands { registry, notice })
}

pub fn register(config: &Config, apps: Arc<AppsProvider>) -> Result<Registration, dupin_core::CommandError> {
    let ConfiguredCommands { registry, notice } = configured(config)?;
    let apps_id = id(APPS);
    let files_id = id(FILES);
    let contents_id = id(CONTENTS);
    let dictionary_id = id(DICTIONARY);
    let file_demotions = Arc::new(RwLock::new(config.file_demotions.clone()));
    let providers: HashMap<_, Arc<dyn Provider>> = [
        (apps_id, apps as Arc<dyn Provider>),
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
    ]
    .into();

    Ok(Registration {
        registry,
        providers,
        notice,
        file_demotions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
