use std::sync::Arc;

use dupin_core::{
    Batch, CommandError, CommandSpec, Config, Item, Payload, Provider, ProviderError, QueryEligibility,
    Request, View,
};
use dupin_js_runtime::Preferences;

use super::extensions::Port;
use super::{Bundled, Capability, Feature, PermissionUse};
use crate::commands::{Configure, ConfiguredCommands, Providers, id, presentation};
use crate::extensions::preferences;
use crate::ui::Ui;

pub(crate) const SNIPPETS: &str = "snippets.search";
/// The bundled extension, in extensions/snippets, that lists the snippets.
const PACKAGE: &str = "snippets";
/// What the broker knows the shipped package by, which alone is handed config.toml's snippets.
const BUNDLED_PACKAGE: &str = "bundled:snippets";
pub(crate) const PASTE_NOTICE: &str = "Copied the snippet. To paste it into the front app as well, allow Dupin in System Settings › Privacy & Security › Accessibility, or turn off Paste snippets in Settings › Search.";

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
    bundled: Some(Bundled {
        package: PACKAGE,
        command: SNIPPETS,
        main_search: false,
        paste_notice: Some(PASTE_NOTICE),
        enabled: |_| true,
        turn_on: |_| {},
    }),
    sources: &[],
};

fn configure(config: &Config, commands: &mut Configure) -> Result<(), CommandError> {
    let command = id(SNIPPETS);
    let spec = CommandSpec::new(
        command.clone(),
        // Every snippet, and the row that adds one.
        dupin_core::snippets::MAX_SNIPPETS + 1,
        presentation(
            "Search snippets",
            Some("Snippets"),
            Some("No snippets yet  ·  ⌫ back to apps"),
            Some("No matching snippets"),
            false,
            0.0,
        ),
    )
    .with_query_eligibility(QueryEligibility::Any);
    // With no snippet at all, the command isn't suggested as its keyword is typed, nor listed
    // by `?`; its keyword still enters it, where the first one is added.
    let none = config.snippets.is_empty() && crate::snippets::library().stored().is_empty();
    commands
        .registry
        .register(if none { spec.unhinted() } else { spec })?;
    commands.add_keywords(&command, &config.snippet_keywords, "snippet_keywords");
    Ok(())
}

fn prepare(config: &Config, commands: &mut Configure) {
    let library = crate::snippets::library();
    let (snippets, notice) =
        dupin_core::snippets::preferences(&config.snippets, &library.stored(), config.snippet_paste);
    commands.snippets = snippets;
    commands.notices.extend(notice);
    commands.notices.extend(
        library
            .problem()
            .map(|problem| format!("Stored snippets couldn't be read: {problem}")),
    );
}

fn start(providers: &mut Providers) {
    let port = providers.port(SNIPPETS);
    providers.add(SNIPPETS, Arc::new(SnippetSearch { port }));
}

fn reload(ui: &mut Ui, config: &Config, commands: &mut ConfiguredCommands) -> bool {
    crate::snippets::library().declare(&config.snippets, config.snippet_paste);
    let changed = hand_over(std::mem::take(&mut commands.snippets));
    changed && ui.session.command_id().as_str() == SNIPPETS
}

/// Hands the snippets, config.toml's and the stored ones, to the bundled package, and says
/// whether they changed.
pub(crate) fn hand_over(snippets: Preferences) -> bool {
    preferences::set(BUNDLED_PACKAGE, snippets)
}

/// Snippet search: the bundled extension's catalogue, which Dupin matches as the user types.
struct SnippetSearch {
    port: Arc<Port>,
}

impl Provider for SnippetSearch {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        self.search_batches(request, &mut |batch| emit(batch.items))
    }

    fn search_batches(&self, request: &Request, emit: &mut dyn FnMut(Batch)) -> Result<(), ProviderError> {
        if request.limit == 0 || request.is_cancelled() {
            return Ok(());
        }
        let Some(extension) = self.port.provider() else {
            emit(vec![unavailable()].into());
            return Ok(());
        };
        let listing = request.query.trim().is_empty();
        extension.search_batches(request, &mut |mut batch| {
            // A catalogue lists itself unranked; snippets used often have always come first.
            if listing {
                for item in &mut batch.items {
                    item.score.get_or_insert(0);
                }
            }
            emit(batch);
        })
    }
}

fn unavailable() -> Item {
    Item {
        payload: Payload::default(),
        id: super::UNAVAILABLE_ROW.into(),
        view: View::Text {
            title: "Snippets aren't available".into(),
            subtitle: "Their extension is off or didn't load; Settings → Extensions says why.".into(),
        },
        record_usage: false,
        icon: None,
        matched: Vec::new(),
        completion: None,
        score: None,
        primary: None,
        alternate: None,
        actions: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Instant;

    use dupin_core::config::Snippet;
    use dupin_core::{Action, Delivery, Effect, History, Session};

    use super::*;
    use crate::extensions::{ExtensionProvider, QUERY_DEADLINE, Role, bundled, load_bundled};

    fn snippet(name: Option<&str>, text: &str) -> Snippet {
        Snippet {
            name: name.map(str::to_owned),
            text: text.into(),
        }
    }

    fn rows(provider: &dyn Provider, query: &str, limit: usize) -> Vec<Item> {
        let mut rows = Vec::new();
        provider
            .search(&Request::detached(query, limit), &mut |batch| rows.extend(batch))
            .unwrap();
        rows
    }

    #[test]
    fn the_command_waits_for_its_bundled_extension() {
        struct Answers;

        impl Provider for Answers {
            fn search(&self, _: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
                emit(vec![Item {
                    id: "snippet:sig".into(),
                    ..unavailable()
                }]);
                Ok(())
            }
        }

        let search = SnippetSearch { port: Arc::default() };
        let ids = |query| -> Vec<(String, Option<i32>)> {
            rows(&search, query, 10)
                .into_iter()
                .map(|item| (item.id, item.score))
                .collect()
        };
        assert_eq!(ids(""), [("unavailable".to_owned(), None)]);
        search.port.set(Some(Arc::new(Answers)));
        assert_eq!(ids(" "), [("snippet:sig".to_owned(), Some(0))], "ranked by usage");
        assert_eq!(ids("sig"), [("snippet:sig".to_owned(), None)]);
    }

    #[test]
    fn the_command_is_suggested_once_there_is_a_snippet_and_its_keyword_always_enters_it() {
        let hinted = |config: &Config| {
            let registry = crate::commands::configured(config).unwrap().registry;
            Session::new(History::default(), registry)
                .unwrap()
                .provider_spec(&id(SNIPPETS))
                .unwrap()
                .hinted()
        };
        assert!(!hinted(&Config::default()));
        let config = Config {
            snippets: BTreeMap::from([("sig".into(), snippet(None, "Best wishes"))]),
            ..Config::default()
        };
        assert!(hinted(&config));
        let registry = crate::commands::configured(&Config::default()).unwrap().registry;
        let mut session = Session::new(History::default(), registry).unwrap();
        session.set_query("snip ");
        assert_eq!(
            session.command_id().as_str(),
            SNIPPETS,
            "where the first one is added"
        );
    }

    // One test, so nothing else changes the package's preferences while it runs.
    #[test]
    fn the_bundled_extension_lists_config_snippets_under_their_command() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let extension = load_bundled(&bundled::directory().unwrap().join(PACKAGE)).unwrap();
        let port = ExtensionProvider::new(extension.clone());
        let mut config = Config {
            snippets: BTreeMap::from([
                (
                    "sig".into(),
                    snippet(Some("Email signature"), "Best wishes,\n  Sherlock"),
                ),
                ("stamp".into(), snippet(None, "Checked on {datetime}")),
                ("two words".into(), snippet(None, "skipped")),
            ]),
            snippet_paste: true,
            ..Config::default()
        };
        let (held, notice) = dupin_core::snippets::preferences(&config.snippets, &[], config.snippet_paste);
        assert!(notice.unwrap().contains("invalid"));
        preferences::set(extension.package.as_str(), held);
        let found = rows(&port, "sig", 10);
        assert_eq!(found.len(), 1);
        let sig = &found[0];
        assert_eq!(
            sig.id, "snippet:sig",
            "the built-in's ID, which usage history keys"
        );
        assert_eq!(
            sig.view,
            View::Text {
                title: "Email signature".into(),
                subtitle: "sig  ·  Best wishes, Sherlock".into(),
            }
        );
        assert_eq!(
            (sig.primary.as_deref(), sig.alternate.as_deref()),
            (Some("paste"), Some("copy"))
        );
        assert_eq!(sig.payload, Payload::text("Best wishes,\n  Sherlock"));
        assert_eq!(
            sig.action("copy"),
            Some(&Action::Snippet {
                text: "Best wishes,\n  Sherlock".into(),
                paste: false,
            })
        );
        let stamp = rows(&port, "stamp", 10).remove(0);
        assert_eq!(stamp.payload, Payload::default(), "filled in only when used");

        config.snippets = (0..64)
            .map(|i| (format!("snippet-{i:02}"), snippet(None, &format!("Example {i}"))))
            .collect();
        config.snippet_paste = false;
        let configured = crate::commands::configured(&config).unwrap();
        preferences::set(extension.package.as_str(), configured.snippets);
        let search = SnippetSearch { port: Arc::default() };
        search.port.set(Some(Arc::new(port)));
        let mut session = Session::new(History::default(), configured.registry).unwrap();
        let effects = session.set_query("snip ");
        let [
            Effect::ReplaceQuery(_),
            Effect::Search {
                command,
                generation,
                query,
                ..
            },
        ] = effects.as_slice()
        else {
            panic!("expected a search in snippets mode, got {effects:?}");
        };
        let limit = session.provider_spec(command).unwrap().candidate_limit();
        search
            .search(&Request::detached(query, limit), &mut |items| {
                session.deliver_from(
                    command.clone(),
                    Delivery::Batch {
                        generation: *generation,
                        items,
                    },
                );
            })
            .unwrap();
        assert_eq!(
            session.items().len(),
            config.snippets.len() + 1,
            "every snippet, and the row that adds one"
        );
        let first = session
            .items()
            .iter()
            .find(|item| item.id.starts_with("snippet:"))
            .unwrap();
        assert_eq!(first.primary.as_deref(), Some("copy"));
        assert!(session.items().iter().any(|item| item.id == "add-snippet"));

        // Stored snippets list beside config.toml's, which keeps a keyword both hold, and only
        // a stored one offers to be edited and deleted.
        let stored = |keyword: &str, name: &str, text: &str| dupin_core::snippets::Snippet {
            keyword: keyword.into(),
            name: Some(name.into()),
            text: text.into(),
        };
        let (held, notice) = dupin_core::snippets::preferences(
            &config.snippets,
            &[
                stored("note", "Meeting note", "Notes for {date}"),
                stored("snippet-01", "Shadowed", "never listed"),
            ],
            false,
        );
        assert!(notice.unwrap().contains("`snippet-01`"));
        preferences::set(extension.package.as_str(), held);
        let titles = |item: &Item| {
            item.actions
                .iter()
                .map(|action| action.title.clone())
                .collect::<Vec<_>>()
        };
        let note = rows(&search, "Meeting", 10).remove(0);
        assert_eq!(note.id, "snippet:note");
        assert_eq!(
            titles(&note),
            [
                "Copy to Clipboard",
                "Edit Snippet",
                "Delete Snippet",
                "Add Snippet"
            ]
        );
        let declared = rows(&search, "snippet-01", 10).remove(0);
        assert_eq!(
            declared.view,
            View::Text {
                title: "snippet-01".into(),
                subtitle: "snippet-01  ·  Example 1".into(),
            }
        );
        assert_eq!(titles(&declared), ["Copy to Clipboard", "Add Snippet"]);

        // What config.toml may hand over always fits the catalogue: as many snippets as it
        // holds, or their whole text budget spent on the longest keywords and names, loaded
        // within the search's deadline.
        let many = (0..=dupin_core::snippets::MAX_SNIPPETS).map(|i| (format!("s{i:05}"), snippet(None, "x")));
        let long = (0..4000).map(|i| (format!("{i:032}"), snippet(Some(&"😀".repeat(80)), "😀")));
        for snippets in [many.collect(), long.collect::<BTreeMap<_, _>>()] {
            config.snippets = snippets;
            config.snippet_paste = true;
            let (held, notice) =
                dupin_core::snippets::preferences(&config.snippets, &[], config.snippet_paste);
            assert!(notice.is_some());
            let listed = held["snippets"].as_array().unwrap();
            let last = listed.last().unwrap()["keyword"].as_str().unwrap().to_owned();
            let count = listed.len();
            preferences::set(extension.package.as_str(), held);
            let found = extension
                .catalogue(
                    &last,
                    10,
                    Instant::now() + QUERY_DEADLINE,
                    &dupin_extension_bridge::Call::none(),
                    Role::Keyword,
                )
                .unwrap_or_else(|error| panic!("{count} snippets: {error:?}"));
            assert_eq!(
                found
                    .iter()
                    .flat_map(|batch| &batch.items)
                    .next()
                    .map(|row| row.id.clone()),
                Some(format!("snippet:{last}")),
                "{count} snippets"
            );
        }
    }
}
