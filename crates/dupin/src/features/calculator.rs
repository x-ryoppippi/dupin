use dupin_core::{CommandError, CommandSpec, Config};

use super::{Feature, default_query};
use crate::commands::{Configure, ConfiguredCommands, Providers, id, presentation};
use crate::ui::Ui;

pub(crate) const CALCULATOR: &str = "calculator.evaluate";

pub(super) const FEATURE: Feature = Feature {
    configure,
    start,
    reload,
    ..Feature::EMPTY
};

fn configure(config: &Config, commands: &mut Configure) -> Result<(), CommandError> {
    let command = id(CALCULATOR);
    commands.registry.register(CommandSpec::new(
        command.clone(),
        usize::from(config.calculator),
        presentation("Calculate", None, None, None, false, 0.0),
    ))?;
    if config.calculator {
        commands.defaults.push(command);
    }
    Ok(())
}

fn start(providers: &mut Providers) {
    providers.add(CALCULATOR, providers.calculator.clone());
}

fn reload(ui: &mut Ui, config: &Config, _: &mut ConfiguredCommands) -> bool {
    ui.currency
        .set_enabled(config.calculator && config.calculator_currency)
        && default_query(ui)
}
