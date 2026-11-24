//! Holds a built-in and the extension that ports it to answering alike: for each declared
//! query, the same rows in the same order, under the same sections, with the same actions.
//! Its latency twin is an implementation beside `native` in scripts/latency/scenarios.toml;
//! a port deletes the native provider only once both hold.

use dupin_core::{Action, Batch, Provider, Request, View};

/// What a person sees of one row: its section, text, and each action's title and effect.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Seen {
    pub section: Option<String>,
    pub title: String,
    pub subtitle: String,
    pub detail: bool,
    pub actions: Vec<(String, String)>,
}

/// A built-in, its extension port and the queries they must answer alike.
pub(crate) struct Parity<'a> {
    pub native: &'a dyn Provider,
    pub extension: &'a dyn Provider,
    pub queries: &'a [&'a str],
    /// Rows asked of each, as the launcher's candidate budget would.
    pub limit: usize,
}

impl Parity<'_> {
    /// Panics with the first query the two answer differently.
    pub fn assert(&self) {
        for query in self.queries {
            let native = seen(self.native, query, self.limit);
            let extension = seen(self.extension, query, self.limit);
            assert_eq!(
                extension, native,
                "the port answers {query:?} unlike the built-in"
            );
        }
    }
}

pub(crate) fn seen(provider: &dyn Provider, query: &str, limit: usize) -> Vec<Seen> {
    let mut batches: Vec<Batch> = Vec::new();
    provider
        .search_batches(&Request::detached(query, limit), &mut |batch| batches.push(batch))
        .expect("providers report failures as rows");
    batches
        .iter()
        .flat_map(|batch| batch.items.iter().map(move |item| (batch, item)))
        .map(|(batch, item)| {
            let (title, subtitle) = match &item.view {
                View::Text { title, subtitle } => (title.clone(), subtitle.clone()),
                View::Detail { title, body } => (title.clone(), body.clone()),
                View::Pair { source, result } => (source.clone(), result.clone()),
            };
            Seen {
                section: batch.section.clone(),
                title,
                subtitle,
                detail: batch.details.contains_key(&item.id),
                actions: item
                    .actions
                    .iter()
                    .map(|action| {
                        let effect = match item.action(&action.id).expect("listed") {
                            Action::Copy(text) => format!("copy {text}"),
                            Action::OpenUrl(url) => format!("open {url}"),
                            Action::Extension {
                                request: dupin_core::ExtensionRequest::Run { action, .. },
                                ..
                            } => format!("run {action}"),
                            Action::Extension {
                                request: dupin_core::ExtensionRequest::Paste { text },
                                ..
                            } => format!("paste {text}"),
                            other => format!("{other:?}"),
                        };
                        (action.title.clone(), effect)
                    })
                    .collect(),
            }
        })
        .collect()
}
