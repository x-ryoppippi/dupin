use crate::platform::system_commands;
use dupin_core::{CommandError, CommandSpec, Config};

use super::{Feature, default_query};
use crate::commands::{Configure, ConfiguredCommands, Providers, id, presentation};
use crate::ui::Ui;

pub(crate) const SYSTEM: &str = "system.commands";

pub(super) const FEATURE: Feature = Feature {
    configure,
    start,
    reload,
    ..Feature::EMPTY
};

fn configure(_: &Config, commands: &mut Configure) -> Result<(), CommandError> {
    let command = id(SYSTEM);
    commands.registry.register(CommandSpec::new(
        command.clone(),
        system_commands::COMMANDS.len(),
        presentation("Search system commands", None, None, None, false, 0.0),
    ))?;
    commands.defaults.push(command);
    Ok(())
}

fn start(providers: &mut Providers) {
    providers.add(SYSTEM, providers.registration.system_provider.clone());
}

fn reload(ui: &mut Ui, config: &Config, _: &mut ConfiguredCommands) -> bool {
    ui.system_provider
        .set_automation_enabled(config.system_automation)
        && (default_query(ui) || ui.session.command_id().as_str() == SYSTEM)
}
