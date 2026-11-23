use std::sync::Arc;

use dupin_core::{CommandError, CommandSpec, Config};
use dupin_mac::privacy::Permission;

use super::{Capability, Feature, PermissionUse};
use crate::commands::{Configure, ConfiguredCommands, Providers, id, presentation};
use crate::ui::Ui;
use crate::window_management::{COMMANDS, WindowProvider};

pub(crate) const WINDOWS: &str = "windows.arrange";

pub(super) const FEATURE: Feature = Feature {
    configure,
    start,
    reload,
    permissions: &[PermissionUse {
        capability: Capability::Permission(Permission::Accessibility),
        label: "window management",
        enabled: |config| config.window_management,
        request_on_enable: true,
    }],
    ..Feature::EMPTY
};

fn configure(config: &Config, commands: &mut Configure) -> Result<(), CommandError> {
    if config.window_management {
        let command = id(WINDOWS);
        commands.registry.register(CommandSpec::new(
            command.clone(),
            COMMANDS.len(),
            presentation("Arrange windows", None, None, None, false, 0.0),
        ))?;
        commands.defaults.push(command);
    }
    Ok(())
}

fn start(providers: &mut Providers) {
    providers.add(WINDOWS, Arc::new(WindowProvider));
}

fn reload(ui: &mut Ui, config: &Config, _: &mut ConfiguredCommands) -> bool {
    ui.window_management_enabled = config.window_management;
    if !config.window_management {
        ui.window_permission_required = false;
    }
    false
}
