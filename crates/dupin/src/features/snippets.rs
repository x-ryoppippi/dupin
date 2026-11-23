use std::sync::Arc;

use dupin_core::snippets::{SnippetCatalog, SnippetProvider};
use dupin_core::{CommandError, CommandSpec, Config, QueryEligibility};

use super::Feature;
use crate::commands::{Configure, ConfiguredCommands, Providers, id, presentation};
use crate::ui::Ui;

pub(crate) const SNIPPETS: &str = "snippets.search";

pub(super) const FEATURE: Feature = Feature {
    configure,
    prepare,
    start,
    reload,
};

fn configure(config: &Config, commands: &mut Configure) -> Result<(), CommandError> {
    let command = id(SNIPPETS);
    commands.registry.register(
        CommandSpec::new(
            command.clone(),
            config.snippets.len(),
            presentation(
                "Search snippets",
                Some("Snippets"),
                Some("No snippets yet: add them to config.toml  ·  ⌫ back to apps"),
                Some("No matching snippets"),
                false,
                0.0,
            ),
        )
        .with_query_eligibility(QueryEligibility::Any),
    )?;
    commands.add_keywords(&command, &config.snippet_keywords, "snippet_keywords");
    Ok(())
}

fn prepare(config: &Config, commands: &mut Configure) {
    let (catalog, notice) = SnippetCatalog::compile(config);
    commands.snippets = catalog;
    commands.notices.extend(notice);
}

fn start(providers: &mut Providers) {
    providers.add(
        SNIPPETS,
        Arc::new(SnippetProvider::new(providers.registration.snippets.clone())),
    );
}

fn reload(ui: &mut Ui, _: &Config, commands: &mut ConfiguredCommands) -> bool {
    let mut snippets = ui.snippets.write().unwrap();
    let next = std::mem::take(&mut commands.snippets);
    let changed = *snippets != next;
    *snippets = next;
    changed && ui.session.command_id().as_str() == SNIPPETS
}
