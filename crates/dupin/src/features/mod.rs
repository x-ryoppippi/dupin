pub(crate) mod aliases;
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
pub(crate) mod snapshots;
pub(super) mod snippets;
pub(super) mod system;
pub(super) mod web;

use dupin_core::{CommandError, Config};
use dupin_extension_bridge::PackageId;
use dupin_mac::privacy::Permission;

use crate::commands::{Configure, ConfiguredCommands, Providers};
use crate::extensions::BUNDLED_PREFIX;
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
    /// The native sources the feature's commands search.
    pub sources: &'static [Source],
}

/// A native source, and what it may be composed with.
pub(super) struct Source {
    /// The name configuration addresses it by.
    pub name: &'static str,
    /// The command it answers.
    pub command: &'static str,
    /// The matcher it starts with, for a source that matches its own candidates; `None` for
    /// one that matches through an index of its own, such as Spotlight.
    pub matcher: Option<&'static str>,
    /// The kind of row its matcher and sorter see, for a source that takes them; what a
    /// package's filter composed onto it may be let see.
    pub rows: Option<dupin_broker::RowKind>,
}

impl Feature {
    const EMPTY: Self = Self {
        configure: |_, _| Ok(()),
        prepare: |_, _| {},
        start: |_| {},
        reload: |_, _, _| false,
        permissions: &[],
        bundled: None,
        sources: &[],
    };
}

/// Every native source the features register, in manifest order.
pub(crate) fn sources() -> impl Iterator<Item = &'static Source> {
    MANIFEST.iter().flat_map(|feature| feature.sources)
}

/// The command the source `name` answers.
pub(super) fn source_command(name: &str) -> &'static str {
    sources()
        .find(|source| source.name == name)
        .unwrap_or_else(|| panic!("no feature registers the source {name}"))
        .command
}

#[derive(Clone, Copy)]
pub(super) struct Bundled {
    /// The package's `name`.
    pub package: &'static str,
    /// The command the feature registers while it is on.
    pub command: &'static str,
    /// For a feature that pastes, what Dupin says when macOS doesn't let it: the feature asks
    /// macOS for Dupin's own access on first paste, as it did before it shipped as a package.
    pub paste_notice: Option<&'static str>,
    /// Whether the feature is on, and so may the package watch on its behalf.
    pub enabled: fn(&Config) -> bool,
    /// Whether the command answers the main search rather than a keyword of its own.
    pub main_search: bool,
}

/// Whether the feature that ships as the extension called `package` is on, if one does.
pub(crate) fn ported_enabled(package: &str, config: &Config) -> Option<bool> {
    bundled(package).map(|bundled| (bundled.enabled)(config))
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

fn bundled(package: &str) -> Option<Bundled> {
    MANIFEST
        .iter()
        .filter_map(|feature| feature.bundled)
        .find(|bundled| bundled.package == package)
}

/// The package that ships as the built-in `command`, if a bundled extension answers it.
pub(crate) fn ported_package(command: &str) -> Option<&'static str> {
    MANIFEST
        .iter()
        .filter_map(|feature| feature.bundled)
        .find(|bundled| bundled.command == command)
        .map(|bundled| bundled.package)
}

/// The built-in command the extension called `package` answers, if a feature ships as it.
pub(crate) fn ported_command(package: &str) -> Option<&'static str> {
    bundled(package).map(|bundled| bundled.command)
}

/// How the extension called `package` answers the built-in command it ports.
pub(crate) fn ported_role(package: &str) -> crate::extensions::Role {
    match bundled(package) {
        Some(Bundled {
            main_search: true, ..
        }) => crate::extensions::Role::MainSearch,
        _ => crate::extensions::Role::Keyword,
    }
}

/// The feature `package` ships, when it is the package Dupin shipped for a built-in rather
/// than a copy in the extensions folder that took its name.
fn shipped(package: &PackageId) -> Option<Bundled> {
    bundled(package.as_str().strip_prefix(BUNDLED_PREFIX)?)
}

/// What Dupin says when it can't paste for the feature `package` ships, if that feature pastes.
pub(crate) fn paste_notice(package: &PackageId) -> Option<&'static str> {
    shipped(package).and_then(|bundled| bundled.paste_notice)
}

/// Whether Dupin fills the placeholders of `package`'s template actions.
pub(crate) fn fills_templates(package: &PackageId) -> bool {
    shipped(package).is_some()
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
    aliases::FEATURE,
    snippets::FEATURE,
    snapshots::FEATURE,
    recent::FEATURE,
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
