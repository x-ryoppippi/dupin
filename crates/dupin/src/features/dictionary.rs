use dupin_core::{CommandError, CommandSpec, Config};

use super::{Feature, Port};
use crate::commands::{Configure, id, presentation};

pub(crate) const DICTIONARY: &str = "dictionary.lookup";

pub(super) const FEATURE: Feature = Feature {
    configure,
    port: Some(Port {
        package: "dictionary",
        command: DICTIONARY,
        title: "Dictionary",
    }),
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
