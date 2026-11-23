use dupin_core::{CommandError, CommandSpec, Config};

use super::Feature;
use crate::commands::{Configure, Providers, id, presentation};

pub(crate) const APPS: &str = "apps.search";

pub(super) const FEATURE: Feature = Feature {
    configure,
    start,
    ..Feature::EMPTY
};

fn configure(_: &Config, commands: &mut Configure) -> Result<(), CommandError> {
    let command = id(APPS);
    commands.registry.register(CommandSpec::new(
        command.clone(),
        50,
        presentation("Search apps", None, None, Some("No matches"), false, 0.0),
    ))?;
    commands.defaults.push(command);
    Ok(())
}

fn start(providers: &mut Providers) {
    providers.add(APPS, providers.apps.clone());
}
