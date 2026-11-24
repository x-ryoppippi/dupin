use std::sync::Arc;

use dupin_core::{
    Batch, CommandError, CommandSpec, Config, Item, Payload, Provider, ProviderError, Request, View,
};

use super::extensions::Port;
use super::{Bundled, Feature};
use crate::commands::{Configure, Providers, id, presentation};

pub(crate) const DICTIONARY: &str = "dictionary.lookup";
/// The bundled extension, in extensions/dictionary, that looks words up.
pub(crate) const PACKAGE: &str = "dictionary";

pub(super) const FEATURE: Feature = Feature {
    configure,
    start,
    bundled: Some(Bundled {
        package: PACKAGE,
        command: DICTIONARY,
        enabled: |_| true,
    }),
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
    let port = providers.port(DICTIONARY);
    providers.add(DICTIONARY, Arc::new(DictionaryLookup { port }));
}

/// Dictionary: the bundled extension's answer, or why there is none.
struct DictionaryLookup {
    port: Arc<Port>,
}

impl Provider for DictionaryLookup {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        self.search_batches(request, &mut |batch| emit(batch.items))
    }

    fn search_batches(&self, request: &Request, emit: &mut dyn FnMut(Batch)) -> Result<(), ProviderError> {
        if request.limit == 0 || request.is_cancelled() {
            return Ok(());
        }
        match self.port.provider() {
            Some(extension) => extension.search_batches(request, emit),
            None => {
                emit(vec![unavailable()].into());
                Ok(())
            }
        }
    }
}

fn unavailable() -> Item {
    Item {
        payload: Payload::default(),
        id: super::UNAVAILABLE_ROW.into(),
        view: View::Text {
            title: "Dictionary isn't available".into(),
            subtitle: "Its bundled extension didn't load; Settings → Extensions says why.".into(),
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
    use super::*;

    struct Answers;

    impl Provider for Answers {
        fn search(&self, _: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
            let mut row = unavailable();
            row.view = View::Text {
                title: "serendipity".into(),
                subtitle: String::new(),
            };
            emit(vec![row]);
            Ok(())
        }
    }

    fn titles(lookup: &DictionaryLookup) -> Vec<String> {
        let mut rows = Vec::new();
        lookup
            .search(&Request::detached("serendipity", 1), &mut |batch| {
                rows.extend(batch)
            })
            .unwrap();
        rows.iter().map(|row| row.display_title().to_owned()).collect()
    }

    #[test]
    fn the_bundled_extension_answers_once_it_has_loaded() {
        let lookup = DictionaryLookup { port: Arc::default() };
        assert_eq!(titles(&lookup), ["Dictionary isn't available"]);
        lookup.port.set(Some(Arc::new(Answers)));
        assert_eq!(titles(&lookup), ["serendipity"]);
        lookup.port.set(None);
        assert_eq!(titles(&lookup), ["Dictionary isn't available"]);
    }
}
