use std::sync::Arc;

use dupin_core::{CommandError, CommandSpec, Config};
use dupin_mac::DictionaryProvider;

use super::Feature;
use crate::commands::{Configure, Providers, id, presentation};

pub(crate) const DICTIONARY: &str = "dictionary.lookup";

pub(super) const FEATURE: Feature = Feature {
    configure,
    start,
    ..Feature::EMPTY
};

fn configure(config: &Config, commands: &mut Configure) -> Result<(), CommandError> {
    let command = id(DICTIONARY);
    commands.registry.register(CommandSpec::new(
        command.clone(),
        1,
        presentation(
            "Look up a word",
            Some("Define"),
            Some("Type a word  ·  ⌫ back to apps"),
            Some("No definition found"),
            true,
            3.0,
        ),
    ))?;
    commands.add_keywords(&command, &config.dictionary_keywords, "dictionary_keywords");
    Ok(())
}

fn start(providers: &mut Providers) {
    providers.add(DICTIONARY, Arc::new(DictionaryProvider::default()));
}
