use std::sync::Arc;
use std::time::{Duration, Instant};

use dupin_core::{Action, Item, ItemAction, Page, Payload, Provider, ProviderError, Request, View};
use dupin_js_runtime::RunError;
use dupin_plugin_protocol as protocol;

use super::Extension;

/// Shorter than the command's session deadline, so a slow guest ends with its own error row
/// rather than the generic expiry.
pub const QUERY_DEADLINE: Duration = Duration::from_secs(2);

pub struct ExtensionProvider {
    extension: Arc<Extension>,
}

impl ExtensionProvider {
    pub fn new(extension: Arc<Extension>) -> Self {
        Self { extension }
    }

    fn problem(&self, message: String) -> Item {
        Item {
            id: "extension-error".into(),
            payload: Payload::default(),
            view: View::Text {
                title: format!("{} failed", self.extension.name),
                subtitle: message,
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
}

fn item(row: protocol::PluginItem) -> Item {
    let protocol::PluginItem {
        id,
        view,
        actions: offered,
    } = row;
    let payload = offered
        .iter()
        .find_map(|action| match &action.effect {
            protocol::Effect::Copy { text } => Some(Payload::text(text.clone())),
            _ => None,
        })
        .unwrap_or_default();
    let actions: Vec<_> = offered
        .into_iter()
        .map(|action| {
            let run = match action.effect {
                protocol::Effect::Copy { text } => Action::Copy(text),
                protocol::Effect::OpenUrl { url } => Action::OpenUrl(url),
                protocol::Effect::Push { view, title, context } => {
                    Action::Push(Page { view, title, context })
                }
            };
            ItemAction::new(&action.id, &action.title, run)
        })
        .collect();
    Item {
        id,
        view,
        payload,
        record_usage: false,
        icon: None,
        matched: Vec::new(),
        completion: None,
        score: None,
        primary: actions.first().map(|action| action.id.clone()),
        alternate: None,
        actions,
    }
}

impl Provider for ExtensionProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        if request.limit == 0 {
            return Ok(());
        }
        let cancellation = request.cancellation();
        let answer = self.extension.query(
            &request.query,
            request.page.as_ref(),
            request.limit,
            Instant::now() + QUERY_DEADLINE,
            move || cancellation.is_cancelled(),
        );
        let items = match answer {
            Ok(rows) => rows.into_iter().map(item).collect(),
            Err(RunError::Cancelled) => return Ok(()),
            Err(RunError::Failed(diagnostic)) => {
                crate::log::warn(&format!("extension {}: {diagnostic}", self.extension.id));
                // A located problem starts with its file name, which keeps its case.
                let subtitle = if diagnostic.file.is_some() {
                    diagnostic.to_string()
                } else {
                    capitalized(&diagnostic.message)
                };
                vec![self.problem(subtitle)]
            }
        };
        emit(items);
        Ok(())
    }
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search(provider: &ExtensionProvider, query: &str) -> Vec<Item> {
        let mut rows = Vec::new();
        provider
            .search(&Request::detached(query, 10), &mut |batch| rows.extend(batch))
            .unwrap();
        rows
    }

    fn row(value: serde_json::Value) -> Item {
        item(serde_json::from_value(value).unwrap())
    }

    #[test]
    fn guest_rows_become_items_whose_first_action_runs_on_enter() {
        let count = row(serde_json::json!(
            {"id": "count", "view": {"kind": "pair", "source": "two words", "result": "2 words"},
             "actions": [
                {"id": "copy", "title": "Copy", "effect": {"kind": "copy", "text": "2"}},
                {"id": "web", "title": "Search", "effect": {"kind": "open_url", "url": "https://example.com/?q=2"}}
             ]}
        ));
        assert_eq!(
            count.view,
            View::Pair {
                source: "two words".into(),
                result: "2 words".into()
            }
        );
        assert_eq!(count.primary.as_deref(), Some("copy"));
        assert_eq!(count.action("copy"), Some(&Action::Copy("2".into())));
        assert_eq!(count.payload, Payload::text("2"));
        assert_eq!(
            count.action("web"),
            Some(&Action::OpenUrl("https://example.com/?q=2".into()))
        );
        assert!(!count.record_usage);

        let inert = row(serde_json::json!(
            {"id": "detail", "view": {"kind": "detail", "title": "Details", "body": "Body"}, "actions": []}
        ));
        assert_eq!(inert.primary, None);
        assert_eq!(inert.payload, Payload::default());
    }

    #[test]
    fn a_page_never_reaches_the_guest() {
        let provider =
            ExtensionProvider::new(Arc::new(crate::extensions::tests::extension("Word Count", "wc")));
        let page = Page {
            view: "stats".into(),
            title: "Statistics".into(),
            context: None,
        };
        let mut rows = Vec::new();
        provider
            .search(&Request::detached_page("", page, 10), &mut |batch| {
                rows.extend(batch)
            })
            .unwrap();
        assert_eq!(
            rows[0].view,
            View::Text {
                title: "Word Count failed".into(),
                subtitle: "Extensions can't open views yet".into(),
            }
        );
    }

    fn javascript(source: &str) -> (tempfile::TempDir, ExtensionProvider) {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(
            folder.path().join(super::super::manifest::PACKAGE_JSON),
            r#"{"name": "word-count", "version": "0.1.0", "dupin": {"title": "Word Count", "keyword": "wc"}}"#,
        )
        .unwrap();
        std::fs::write(folder.path().join("main.js"), source).unwrap();
        let extension = super::super::load_folder(folder.path()).unwrap();
        (folder, ExtensionProvider::new(extension))
    }

    #[test]
    fn the_word_counter_counts_words_and_copies_the_count() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions/word-count-js");
        let extension = super::super::load_folder(&root).unwrap();
        assert_eq!(extension.keyword, "wc");
        let rows = search(&ExtensionProvider::new(extension), "two words");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].view,
            View::Text {
                title: "2 words".into(),
                subtitle: String::new()
            }
        );
        assert_eq!(rows[0].primary.as_deref(), Some("copy"));
        assert_eq!(rows[0].action("copy"), Some(&Action::Copy("2".into())));
        assert_eq!(rows[0].payload, Payload::text("2"));
        assert!(!rows[0].record_usage);
    }

    #[test]
    fn javascript_urls_open_first_and_text_copies() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let (_folder, provider) = javascript(
            "export function search(query) { return [{ id: 'docs', title: 'Docs', subtitle: query, url: 'https://example.com/?q=2', text: '2' }]; }",
        );
        let rows = search(&provider, "two words");
        assert_eq!(rows[0].id, "docs");
        assert_eq!(rows[0].primary.as_deref(), Some("open"));
        assert_eq!(
            rows[0].action("open"),
            Some(&Action::OpenUrl("https://example.com/?q=2".into()))
        );
        assert_eq!(rows[0].action("copy"), Some(&Action::Copy("2".into())));
        assert_eq!(rows[0].payload, Payload::text("2"));
    }

    #[test]
    fn javascript_failures_become_a_visible_error_row_with_the_location() {
        dupin_extension_host::skip_inside_a_sandbox!();
        for (source, subtitle) in [
            (
                "export function search() {\n  throw new Error('boom');\n}",
                "main.js:2:13: search threw Error: boom",
            ),
            (
                "export function search() { return [{ title: 'x', url: 'file:///' }]; }",
                "main.js: `items[0].url` must be an http or https URL with a host",
            ),
        ] {
            let (_folder, provider) = javascript(source);
            let rows = search(&provider, "query");
            assert_eq!(rows.len(), 1);
            assert_eq!(
                rows[0].view,
                View::Text {
                    title: "Word Count failed".into(),
                    subtitle: subtitle.into(),
                }
            );
            assert!(rows[0].actions.is_empty());
        }
    }
}
