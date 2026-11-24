use std::sync::Arc;

use dupin_core::{CommandError, CommandSpec, Config};
use dupin_mac::DictionaryProvider;

use super::Feature;
use crate::commands::{Configure, Providers, id, presentation};

pub(crate) const DICTIONARY: &str = "dictionary.lookup";

pub(super) const FEATURE: Feature = Feature {
    configure,
    start,
    ..Feature::EMPTY
};

fn configure(config: &Config, commands: &mut Configure) -> Result<(), CommandError> {
    let command = id(DICTIONARY);
    commands.registry.register(CommandSpec::new(
        command.clone(),
        1,
        presentation(
            "Look up a word",
            Some("Define"),
            Some("Type a word  ·  ⌫ back to apps"),
            Some("No definition found"),
            true,
            3.0,
        ),
    ))?;
    commands.add_keywords(&command, &config.dictionary_keywords, "dictionary_keywords");
    Ok(())
}

fn start(providers: &mut Providers) {
    let provider = if rune::tooling::is_headless() {
        DictionaryProvider::fixtures()
    } else {
        DictionaryProvider::default()
    };
    providers.add(DICTIONARY, Arc::new(provider));
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use dupin_core::{Batch, Detail, Provider, Request};
    use dupin_extension_bridge::{Bridge, Broker, Services};
    use dupin_mac::DictionaryProvider;

    use crate::extensions::{ExtensionProvider, Role, access, bundled, load_folder};

    /// Each row's title, detail and dict:// link.
    fn shown(provider: &dyn Provider, query: &str) -> Vec<(String, Option<Arc<Detail>>, Option<String>)> {
        let mut batches: Vec<Batch> = Vec::new();
        provider
            .search_batches(&Request::detached(query, 1), &mut |batch| batches.push(batch))
            .unwrap();
        batches
            .iter()
            .flat_map(|batch| {
                batch.items.iter().map(|item| {
                    let link = item
                        .actions
                        .iter()
                        .find_map(|action| match item.action(&action.id) {
                            Some(dupin_core::Action::OpenUrl(url)) => Some(url.clone()),
                            _ => None,
                        });
                    (
                        item.display_title().to_owned(),
                        batch.details.get(&item.id).cloned(),
                        link,
                    )
                })
            })
            .collect()
    }

    #[test]
    fn the_bundled_extension_lays_entries_out_as_the_native_provider_does() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let folder = bundled::find(&bundled::directory().unwrap(), "dictionary").unwrap();
        let extension = load_folder(&folder).unwrap();
        let mut services = Services::none();
        services.dictionary = access::fixed_dictionary();
        let bridge = Bridge::new(
            Broker::new(Default::default(), Default::default()),
            None,
            services,
        );
        bridge.set_bundled(&extension.package, extension.revision.clone());
        bridge.install(
            &extension.package,
            extension.revision.clone(),
            extension.capabilities.clone(),
        );
        let port = ExtensionProvider::with_bridge(extension, Role::Keyword, bridge);
        let native = DictionaryProvider::fixtures();
        for query in ["run", "serendipity", " run ", "unknown"] {
            assert_eq!(shown(&port, query), shown(&native, query), "{query:?}");
        }
        assert_eq!(shown(&port, "run").len(), 1);
    }
}
