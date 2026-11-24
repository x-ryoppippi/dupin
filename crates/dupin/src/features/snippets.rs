use std::sync::Arc;

use dupin_core::snippets::{SnippetCatalog, SnippetProvider};
use dupin_core::{CommandError, CommandSpec, Config, QueryEligibility};

use super::{Capability, Feature, PermissionUse};
use crate::commands::{Configure, ConfiguredCommands, Providers, id, presentation};
use crate::ui::Ui;

pub(crate) const SNIPPETS: &str = "snippets.search";

pub(super) const FEATURE: Feature = Feature {
    configure,
    prepare,
    start,
    reload,
    permissions: &[PermissionUse {
        capability: Capability::PostEvent,
        label: "snippet pasting",
        enabled: |config| config.snippet_paste,
        request_on_enable: true,
    }],
    bundled: None,
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::RwLock;

    use dupin_core::config::Snippet;

    use super::*;
    use crate::extensions::parity::Parity;
    use crate::extensions::{ExtensionProvider, bundled, load_folder, preferences};

    fn config(paste: bool) -> Config {
        let snippet = |name: Option<&str>, text: &str| Snippet {
            name: name.map(str::to_owned),
            text: text.into(),
        };
        Config {
            snippets: BTreeMap::from([
                (
                    "addr".into(),
                    snippet(Some("Home address"), "221B Baker Street, London NW1 6XE"),
                ),
                (
                    "sig".into(),
                    snippet(Some("Email signature"), "Best wishes,\n  Sherlock"),
                ),
                (
                    "stamp".into(),
                    snippet(Some("Checked stamp"), "Checked on {datetime}"),
                ),
                (
                    "quote".into(),
                    snippet(Some("Quote the clipboard"), "> {clipboard}"),
                ),
                ("today".into(), snippet(None, "{date}")),
                ("long".into(), snippet(None, &"word\t".repeat(40))),
                ("nihongo".into(), snippet(Some("挨拶"), "よろしく　お願いします")),
                ("two words".into(), snippet(None, "skipped")),
            ]),
            snippet_paste: paste,
            ..Config::default()
        }
    }

    // One test, so nothing else changes the package's preferences while it runs.
    #[test]
    fn the_bundled_extension_answers_like_the_native_snippets() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let extension = load_folder(&bundled::directory().unwrap().join("snippets")).unwrap();
        let port = ExtensionProvider::new(extension.clone());
        for paste in [false, true] {
            let config = config(paste);
            let (catalog, _) = SnippetCatalog::compile(&config);
            let native = SnippetProvider::new(Arc::new(RwLock::new(catalog)));
            let (held, notice) = dupin_core::snippets::preferences(&config);
            assert!(notice.is_some());
            preferences::set(extension.package.as_str(), held);
            Parity {
                native: &native,
                extension: &port,
                queries: &[
                    "", " ", "s", "sig", "email", "e", "a", "addr", "today", "stamp", "check", "quote",
                    "clip", "long", "word", "nihon", "挨拶", "xyz",
                ],
                limit: config.snippets.len(),
            }
            .assert();
        }
    }
}
