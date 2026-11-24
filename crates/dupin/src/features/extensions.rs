use std::sync::Arc;

use dupin_core::{CommandId, CommandSpec, Config};
use dupin_host_services::{calendar, clipboard};
use dupin_mac::privacy::Permission;

use super::{Capability, Feature, PermissionUse};
use crate::commands::{ConfiguredCommands, presentation};
use crate::extensions::{Extension, ExtensionProvider, QUERY_DEADLINE};
use crate::ui::Ui;

pub(super) const FEATURE: Feature = Feature {
    reload,
    // An extension's own grant decides whether it may use the access, so turning extensions on
    // never asks macOS.
    permissions: &[
        PermissionUse {
            capability: Capability::Permission(Permission::of(calendar::PERMISSION)),
            label: "extensions that read calendars",
            enabled: |config| config.extensions,
            request_on_enable: false,
        },
        PermissionUse {
            capability: Capability::Permission(Permission::of(clipboard::PERMISSION)),
            label: "extensions that read the clipboard",
            enabled: |config| config.extensions,
            request_on_enable: false,
        },
    ],
    ..Feature::EMPTY
};

fn reload(ui: &mut Ui, config: &Config, commands: &mut ConfiguredCommands) -> bool {
    ui.watch_extensions(config.extensions);
    ui.install_extensions(commands)
}

pub(crate) fn provider(
    extension: Arc<Extension>,
    bridge: dupin_extension_bridge::Bridge,
) -> Arc<ExtensionProvider> {
    Arc::new(ExtensionProvider::with_bridge(extension, bridge))
}

/// Extensions whose commands were registered, and why the others were not. Built-in and
/// configured keywords always win: an extension never replaces another command's keyword.
pub struct Extensions {
    pub registered: Vec<(CommandId, Arc<Extension>)>,
    pub problems: Vec<String>,
}

pub fn add_extensions(configured: &mut ConfiguredCommands, extensions: &[Arc<Extension>]) -> Extensions {
    let mut added = Extensions {
        registered: Vec::new(),
        problems: Vec::new(),
    };
    for extension in extensions {
        let keyword = &extension.keyword;
        if configured.web.has_keyword(keyword) || configured.registry.keyword(keyword).is_some() {
            added.problems.push(format!(
                "{}: keyword `{keyword}` is already used by another command",
                extension.folder_name
            ));
            continue;
        }
        let Ok(command) = CommandId::new(format!("extension.{}", extension.id)) else {
            continue;
        };
        let mut spec = CommandSpec::new(
            command.clone(),
            50,
            presentation(
                &format!("Ask {}", extension.name),
                Some(&extension.name),
                Some(&format!("Type for {}  ·  ⌫ back to apps", extension.name)),
                Some("No results"),
                true,
                0.0,
            ),
        );
        spec.deadline = QUERY_DEADLINE + std::time::Duration::from_secs(1);
        if configured.registry.register(spec).is_ok()
            && configured.registry.add_keyword(&command, keyword.clone()).is_ok()
        {
            added.registered.push((command, extension.clone()));
        }
    }
    added
}
