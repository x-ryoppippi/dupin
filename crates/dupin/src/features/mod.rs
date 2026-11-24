pub(super) mod apps;
pub(super) mod calculator;
pub(super) mod calendar;
pub(super) mod clipboard;
pub(super) mod dictionary;
pub(super) mod extensions;
pub(super) mod files;
mod launcher;
pub(super) mod recent;
mod scripts;
mod settings;
pub(super) mod snippets;
pub(super) mod system;
pub(super) mod web;
pub(super) mod windows;

use dupin_core::{CommandError, Config};
use dupin_mac::privacy::Permission;

use crate::commands::{Configure, ConfiguredCommands, Providers};
use crate::ui::Ui;

pub(super) struct Feature {
    pub configure: fn(&Config, &mut Configure) -> Result<(), CommandError>,
    pub prepare: fn(&Config, &mut Configure),
    pub start: fn(&mut Providers),
    pub reload: fn(&mut Ui, &Config, &mut ConfiguredCommands) -> bool,
    permissions: &'static [PermissionUse],
    /// The bundled extension that answers the feature's command, which then never answers a
    /// keyword of its own.
    bundled: Option<Bundled>,
}

impl Feature {
    const EMPTY: Self = Self {
        configure: |_, _| Ok(()),
        prepare: |_, _| {},
        start: |_| {},
        reload: |_, _, _| false,
        permissions: &[],
        bundled: None,
    };
}

#[derive(Clone, Copy)]
pub(super) struct Bundled {
    /// The package's `name`.
    pub package: &'static str,
    /// The command the feature registers while it is on.
    pub command: &'static str,
    /// Whether the feature is on, and so may the package watch on its behalf.
    pub enabled: fn(&Config) -> bool,
}

/// Whether the feature that ships as the extension called `package` is on, if one does.
pub(crate) fn ported_enabled(package: &str, config: &Config) -> Option<bool> {
    MANIFEST
        .iter()
        .filter_map(|feature| feature.bundled)
        .find(|bundled| bundled.package == package)
        .map(|bundled| (bundled.enabled)(config))
}

/// The row a built-in command that a bundled extension answers shows while the package is off
/// or didn't load.
pub(crate) const UNAVAILABLE_ROW: &str = "unavailable";

/// Whether the row `id` that `command` shows is its feature's answer, not one Dupin shows in
/// place of the bundled extension that answers the command.
pub(crate) fn answered_by_feature(command: &str, id: &str) -> bool {
    let ported = MANIFEST
        .iter()
        .filter_map(|feature| feature.bundled)
        .any(|bundled| bundled.command == command);
    !ported || (id != UNAVAILABLE_ROW && !crate::extensions::stands_in(id))
}

/// The built-in command the extension called `package` answers, if a feature ships as it.
pub(crate) fn ported_command(package: &str) -> Option<&'static str> {
    MANIFEST
        .iter()
        .filter_map(|feature| feature.bundled)
        .find(|bundled| bundled.package == package)
        .map(|bundled| bundled.command)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Capability {
    Permission(Permission),
    PostEvent,
}

impl Capability {
    pub(super) fn settings_permission(self) -> Permission {
        match self {
            Self::Permission(permission) => permission,
            Self::PostEvent => Permission::Accessibility,
        }
    }
}

pub(super) struct PermissionUse {
    pub capability: Capability,
    pub label: &'static str,
    pub enabled: fn(&Config) -> bool,
    pub request_on_enable: bool,
}

// Default providers are appended in this order. Built-in routes are registered before
// configured aliases, and web searches are compiled only after both have reserved their keywords.
pub(super) const MANIFEST: &[Feature] = &[
    // Dynamic routes are reconciled against the new registry before live catalogues are swapped.
    extensions::FEATURE,
    launcher::FEATURE,
    files::FEATURE,
    clipboard::FEATURE,
    calendar::FEATURE,
    dictionary::FEATURE,
    calculator::FEATURE,
    apps::FEATURE,
    system::FEATURE,
    scripts::FEATURE,
    settings::FEATURE,
    web::FEATURE,
    snippets::FEATURE,
    recent::FEATURE,
    windows::FEATURE,
];

pub(super) fn permission_uses() -> impl Iterator<Item = &'static PermissionUse> {
    MANIFEST.iter().flat_map(|feature| feature.permissions)
}

pub(super) fn reload(ui: &mut Ui, config: &Config, commands: &mut ConfiguredCommands) -> bool {
    let mut refresh = false;
    for feature in MANIFEST {
        refresh |= (feature.reload)(ui, config, commands);
    }
    refresh
}

fn default_query(ui: &Ui) -> bool {
    ui.session.command_id().as_str() == apps::APPS && !ui.session.query().trim().is_empty()
}
