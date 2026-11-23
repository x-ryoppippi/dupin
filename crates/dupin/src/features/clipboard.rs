use std::sync::Arc;

use dupin_core::clipboard::ClipboardProvider;
use dupin_core::{CommandError, CommandSpec, Config, QueryEligibility};

use super::Feature;
use crate::commands::{Configure, ConfiguredCommands, Providers, id, presentation};
use crate::ui::Ui;

pub(crate) const CLIPBOARD: &str = "clipboard.history";

pub(super) const FEATURE: Feature = Feature {
    configure,
    start,
    reload,
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
    ui.clipboard.apply(&config.clipboard) && ui.session.command_id().as_str() == CLIPBOARD
}
