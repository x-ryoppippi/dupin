use std::sync::Arc;

use dupin_core::clipboard::ClipboardProvider;
use dupin_core::{CommandError, CommandSpec, Config, QueryEligibility};
use dupin_mac::privacy::Permission;

use super::{Capability, Feature, PermissionUse};
use crate::commands::{Configure, ConfiguredCommands, Providers, id, presentation};
use crate::ui::Ui;

pub(crate) const CLIPBOARD: &str = "clipboard.history";

pub(super) const FEATURE: Feature = Feature {
    configure,
    start,
    reload,
    permissions: &[PermissionUse {
        capability: Capability::Permission(Permission::Pasteboard),
        label: "clipboard history",
        enabled: |config| config.clipboard.enabled,
        // `clip` offers the request itself, so turning the history on only checks access.
        request_on_enable: false,
    }],
    ..Feature::EMPTY
};

fn configure(config: &Config, commands: &mut Configure) -> Result<(), CommandError> {
    if config.clipboard.enabled {
        let clipboard = id(CLIPBOARD);
        let mut spec = CommandSpec::new(
            clipboard.clone(),
            config.clipboard.max_entries,
            presentation(
                "Search clipboard history",
                Some("Clipboard"),
                Some("Nothing copied yet  ·  ⌫ back to apps"),
                Some("No matches"),
                false,
                4.0,
            ),
        )
        .with_query_eligibility(QueryEligibility::Any);
        spec.private_results = true;
        commands.registry.register(spec)?;
        commands.registry.add_keyword(&clipboard, "clip")?;
    }
    Ok(())
}

fn start(providers: &mut Providers) {
    providers.add(
        CLIPBOARD,
        Arc::new(ClipboardProvider::new(
            providers.clipboard.clone(),
            dupin_mac::clipboard::access_item,
        )),
    );
}

fn reload(ui: &mut Ui, config: &Config, _: &mut ConfiguredCommands) -> bool {
    if crate::extensions::preferences::choose(crate::clipboard::PACKAGE, preferences(&config.clipboard)) {
        ui.preferences_changed(crate::clipboard::PACKAGE);
    }
    ui.clipboard.apply(&config.clipboard) && ui.session.command_id().as_str() == CLIPBOARD
}

/// The history's Settings rows and `[clipboard]` keys, as the bundled package's preferences.
fn preferences(config: &dupin_core::config::Clipboard) -> serde_json::Map<String, serde_json::Value> {
    serde_json::Map::from_iter([
        (
            "retention".into(),
            serde_json::to_value(config.retention).unwrap_or_default(),
        ),
        ("includeImages".into(), config.include_images.into()),
        ("includeFiles".into(), config.include_files.into()),
        ("maxEntries".into(), config.max_entries.into()),
    ])
}
