use std::sync::Arc;

use dupin_core::{CommandError, CommandSpec, Config};

use super::Feature;
use crate::commands::{Configure, Providers, id, presentation};

const SETTINGS: &str = "settings.search";

pub(super) const FEATURE: Feature = Feature {
    configure,
    start,
    ..Feature::EMPTY
};

fn configure(_: &Config, commands: &mut Configure) -> Result<(), CommandError> {
    let command = id(SETTINGS);
    commands.registry.register(CommandSpec::new(
        command.clone(),
        10,
        presentation("Search settings", None, None, None, false, 0.0),
    ))?;
    commands.defaults.push(command);
    Ok(())
}

fn start(providers: &mut Providers) {
    providers.add(SETTINGS, Arc::new(crate::platform::SettingsProvider::default()));
}
