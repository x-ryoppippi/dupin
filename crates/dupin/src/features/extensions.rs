use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use dupin_core::{CommandId, CommandSpec, Config, Provider, QueryEligibility};
use dupin_extension_bridge::OsPermission;
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
        PermissionUse {
            capability: Capability::Permission(Permission::of(OsPermission::Accessibility)),
            label: "extensions that paste or arrange windows",
            enabled: |config| config.extensions,
            request_on_enable: false,
        },
    ],
    ..Feature::EMPTY
};

/// Which provider answers a built-in command that a bundled extension ports: the extension
/// that runs now, handed in whenever the extensions are registered again.
#[derive(Default)]
pub(crate) struct Port(RwLock<Option<Arc<dyn Provider>>>);

impl Port {
    pub(crate) fn set(&self, provider: Option<Arc<dyn Provider>>) {
        *self.0.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = provider;
    }

    pub(crate) fn provider(&self) -> Option<Arc<dyn Provider>> {
        self.0
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// Gives `provider`, the extension registered for `command`, to the built-in that answers
/// `command` through a port, or back for a searcher of its own.
pub(crate) fn hand_over(
    ports: &HashMap<CommandId, Arc<Port>>,
    command: &CommandId,
    provider: Arc<dyn Provider>,
) -> Option<Arc<dyn Provider>> {
    match ports.get(command) {
        Some(port) => {
            port.set(Some(provider));
            None
        }
        None => Some(provider),
    }
}

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
/// configured keywords, including config.toml's web search aliases, always win: an extension
/// never replaces another command's keyword.
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
        // A feature that ships as this extension owns its command and keyword, and the
        // extension answers that command only while the feature is on.
        if let Some(command) = super::ported_command(&extension.id) {
            if configured.registry.ids().iter().any(|id| id == command) {
                added
                    .registered
                    .push((id(command), extension.clone(), super::ported_role(&extension.id)));
            }
            continue;
        }
        let keyword = &extension.keyword;
        let holder = configured.registry.holder(keyword).map(|holder| match holder {
            dupin_core::KeywordHolder::Web(_) => "a web search in config.toml",
            dupin_core::KeywordHolder::Command(_) => "another command",
            dupin_core::KeywordHolder::Path(_) => "an alias in config.toml",
        });
        let holder = holder.or_else(|| {
            configured
                .package_aliases
                .iter()
                .any(|(alias, _)| alias == keyword)
                .then_some("an alias in config.toml")
        });
        if let Some(holder) = holder {
            added.problems.push(format!(
                "{}: keyword `{keyword}` is already used by {holder}",
                extension.folder_name
            ));
            // The package behind the built-in web searches still lists them without its keyword.
            if extension.package.as_str() != crate::web::WEB_SEARCH_PACKAGE {
                continue;
            }
        } else {
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
                for (alias, _) in configured
                    .package_aliases
                    .iter()
                    .filter(|(_, package)| *package == extension.id)
                {
                    if let Err(error) = configured.registry.add_keyword(&command, alias.clone()) {
                        added.problems.push(format!("alias `{alias}`: {error}"));
                    }
                }
                added.registered.push((command, extension.clone(), Role::Keyword));
            }
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
    for (alias, package) in &configured.package_aliases {
        let reached = added
            .registered
            .iter()
            .any(|(_, extension, role)| *role == Role::Keyword && extension.id == *package);
        if !reached {
            added.problems.push(format!(
                "alias `{alias}` names `{package}`, which is neither a command nor an extension that is on"
            ));
        }
    }
    added
}
