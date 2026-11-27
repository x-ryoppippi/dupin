use std::sync::Arc;

use dupin_core::{CommandError, CommandSpec, Config};

use super::Feature;
use crate::commands::{Configure, Providers, id, presentation};
use crate::scripts::provider::ScriptsProvider;

const SCRIPTS: &str = "scripts.search";

pub(super) const FEATURE: Feature = Feature {
    configure,
    start,
    ..Feature::EMPTY
};

fn configure(_: &Config, commands: &mut Configure) -> Result<(), CommandError> {
    let command = id(SCRIPTS);
    commands.registry.register(CommandSpec::new(
        command.clone(),
        8,
        presentation("Search scripts", None, None, None, false, 0.0),
    ))?;
    commands.defaults.push(command);
    Ok(())
}

fn start(providers: &mut Providers) {
    providers.add(
        SCRIPTS,
        Arc::new(ScriptsProvider(providers.registration.scripts.clone())),
    );
}
