use std::sync::Arc;

use dupin_core::{CommandId, CommandSpec, Config, QueryEligibility};
use dupin_host_services::{calendar, clipboard};
use dupin_mac::privacy::Permission;

use super::apps::APPS;
use super::{Capability, Feature, PermissionUse};
use crate::commands::{ConfiguredCommands, id, presentation};
use crate::extensions::{Extension, ExtensionProvider, QUERY_DEADLINE, Role};
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

/// Rows one extension's catalogue adds to the main search.
const DEFAULT_SEARCH_ROWS: usize = 8;

fn reload(ui: &mut Ui, config: &Config, commands: &mut ConfiguredCommands) -> bool {
    ui.watch_extensions(config.extensions);
    ui.install_extensions(commands)
}

pub(crate) fn provider(
    extension: Arc<Extension>,
    role: Role,
    bridge: dupin_extension_bridge::Bridge,
) -> Arc<ExtensionProvider> {
    Arc::new(ExtensionProvider::with_bridge(extension, role, bridge))
}

/// Extensions whose commands were registered, and why the others were not. Built-in and
/// configured keywords always win: an extension never replaces another command's keyword. An
/// extension's keyword wins over a web search's alias.
pub struct Extensions {
    pub registered: Vec<(CommandId, Arc<Extension>, Role)>,
    pub problems: Vec<String>,
}

pub fn add_extensions(configured: &mut ConfiguredCommands, extensions: &[Arc<Extension>]) -> Extensions {
    let mut added = Extensions {
        registered: Vec::new(),
        problems: Vec::new(),
    };
    // Extensions join the main search after apps, in the order of their folders.
    let mut after = id(APPS);
    for extension in extensions {
        let keyword = &extension.keyword;
        if configured.registry.keyword(keyword).is_some() {
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
        )
        // An `items()` catalogue lists itself before anything is typed; `search` still only
        // runs for a query.
        .with_query_eligibility(QueryEligibility::Any);
        spec.deadline = QUERY_DEADLINE + std::time::Duration::from_secs(1);
        if configured.registry.register(spec).is_ok()
            && configured.registry.add_keyword(&command, keyword.clone()).is_ok()
        {
            added.registered.push((command, extension.clone(), Role::Keyword));
        }
        if !extension.default_search {
            continue;
        }
        let Ok(command) = CommandId::new(format!("extension-search.{}", extension.id)) else {
            continue;
        };
        let mut spec = CommandSpec::new(
            command.clone(),
            DEFAULT_SEARCH_ROWS,
            presentation("Search apps", None, None, Some("No matches"), false, 0.0),
        );
        spec.deadline = QUERY_DEADLINE + std::time::Duration::from_secs(1);
        if configured.registry.register(spec).is_ok()
            && configured
                .registry
                .add_default_provider(command.clone(), &after)
                .is_ok()
        {
            after = command.clone();
            added
                .registered
                .push((command, extension.clone(), Role::DefaultSearch));
        }
    }
    added
}
