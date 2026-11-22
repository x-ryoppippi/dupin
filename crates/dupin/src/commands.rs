use std::collections::HashMap;
use std::sync::Arc;

use dupin_core::{
    CommandId, CommandRegistry, CommandSpec, Config, Provider, QueryPresentation, ResultLayout,
};
use dupin_mac::{AppsProvider, DictionaryProvider, SpotlightProvider, Target};

pub const APPS: &str = "apps.search";
pub const FILES: &str = "files.names";
pub const CONTENTS: &str = "files.contents";
pub const DICTIONARY: &str = "dictionary.lookup";

pub struct Registration {
    pub registry: CommandRegistry,
    pub providers: HashMap<CommandId, Arc<dyn Provider>>,
    pub notice: Option<String>,
}

pub struct ConfiguredCommands {
    pub registry: CommandRegistry,
    pub notice: Option<String>,
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
    result_layout: ResultLayout,
    backdrop: f32,
) -> QueryPresentation {
    QueryPresentation {
        placeholder: placeholder.into(),
        chip: chip.map(str::to_owned),
        empty_hint: empty_hint.map(str::to_owned),
        no_results_hint: no_results_hint.map(str::to_owned),
        show_searching,
        result_layout,
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
            presentation(
                "Search apps",
                None,
                None,
                Some("No matches"),
                false,
                ResultLayout::List,
                0.0,
            ),
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
                ResultLayout::List,
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
                ResultLayout::List,
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
                ResultLayout::Detail,
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
    ]
    .into();

    Ok(Registration {
        registry,
        providers,
        notice,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
