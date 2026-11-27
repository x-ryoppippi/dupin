use dupin_core::{Action, AppCommand, CommandError, CommandSpec, Config, SystemCommand};
use dupin_mac::system_commands;

use super::{Feature, default_query};
use crate::commands::{Configure, ConfiguredCommands, Providers, id, presentation};
use crate::ui::Ui;

pub(crate) const SYSTEM: &str = "system.commands";

/// The name a binding gives the system commands and Dupin's own, as `system/<row>`.
pub(crate) const BINDING_NAME: &str = "system";

/// Each system command a binding can run, by the row a binding names it by.
pub(crate) const ROWS: [(&str, Action); 10] = [
    ("sleep", Action::System(SystemCommand::Sleep)),
    ("lock-screen", Action::System(SystemCommand::LockScreen)),
    ("sleep-displays", Action::System(SystemCommand::SleepDisplays)),
    ("toggle-dark-mode", Action::System(SystemCommand::ToggleDarkMode)),
    ("restart", Action::System(SystemCommand::Restart)),
    ("shut-down", Action::System(SystemCommand::ShutDown)),
    ("log-out", Action::System(SystemCommand::LogOut)),
    ("empty-trash", Action::System(SystemCommand::EmptyTrash)),
    ("restart-dupin", Action::App(AppCommand::Relaunch)),
    ("update-dupin", Action::App(AppCommand::Update)),
];

/// The system command the row `row` names, as its search row shows it.
pub(crate) struct Row {
    pub action: Action,
    pub title: &'static str,
    pub subtitle: &'static str,
    /// Whether choosing it asks first.
    pub confirms: bool,
}

pub(crate) fn row(row: &str) -> Option<Row> {
    let (_, action) = ROWS.iter().find(|(name, _)| *name == row)?;
    let (title, subtitle, confirms) = match action {
        Action::System(command) => {
            let (title, subtitle, prompt) = system_commands::labels(*command);
            (title, subtitle, prompt.is_some())
        }
        Action::App(command) => {
            let (title, subtitle, _) = super::app::labels(*command);
            (title, subtitle, false)
        }
        _ => unreachable!("system rows run system and app commands"),
    };
    Some(Row {
        action: action.clone(),
        title,
        subtitle,
        confirms,
    })
}

/// Why the row `row` can't run now, as a binding names it, with System Command Automation
/// `automation` and updates `updates` as this build has them.
pub(crate) fn unavailable(row: &str, automation: bool, updates: bool) -> Option<String> {
    let address = format!("{BINDING_NAME}/{row}");
    match self::row(row).map(|row| row.action) {
        None => Some(format!("names `{address}`, which is no system command")),
        Some(Action::System(command)) if !automation && system_commands::needs_automation(command) => Some(
            format!("names `{address}`, which runs only with System Command Automation on"),
        ),
        Some(Action::App(AppCommand::Update)) if !updates => {
            Some(format!("names `{address}`, which this build doesn't offer"))
        }
        Some(_) => None,
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_system_command_has_a_row_a_binding_names() {
        for command in system_commands::COMMANDS {
            let (title, _, prompt) = system_commands::labels(command);
            let (name, _) = ROWS
                .iter()
                .find(|(_, action)| *action == Action::System(command))
                .unwrap_or_else(|| panic!("{title} has no row"));
            let row = row(name).unwrap();
            assert_eq!((row.title, row.confirms), (title, prompt.is_some()));
            assert_eq!(*name, title.to_lowercase().replace(' ', "-"));
        }
        assert_eq!(row("restart-dupin").unwrap().title, "Restart Dupin");
        assert!(row("Sleep").is_none());
    }

    #[test]
    fn a_row_that_cant_run_says_why() {
        assert_eq!(unavailable("sleep", false, false), None);
        assert_eq!(
            unavailable("shut-down", false, true).unwrap(),
            "names `system/shut-down`, which runs only with System Command Automation on"
        );
        assert_eq!(unavailable("shut-down", true, true), None);
        assert_eq!(
            unavailable("update-dupin", true, false).unwrap(),
            "names `system/update-dupin`, which this build doesn't offer"
        );
        assert_eq!(
            unavailable("nap", true, true).unwrap(),
            "names `system/nap`, which is no system command"
        );
    }
}
