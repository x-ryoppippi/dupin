use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use dupin_core::{Action, Item, ItemAction, Payload, Provider, ProviderError, Request, View};
use dupin_plugin_protocol as protocol;
use dupin_plugin_runtime::{HostError, QueryControl};

use super::Extension;

/// Shorter than the command's session deadline, so a slow guest ends with its own error row
/// rather than the generic expiry.
pub const QUERY_DEADLINE: Duration = Duration::from_secs(2);

pub struct ExtensionProvider {
    extension: Arc<Extension>,
    requests: AtomicU64,
}

impl ExtensionProvider {
    pub fn new(extension: Arc<Extension>) -> Self {
        Self {
            extension,
            requests: AtomicU64::new(0),
        }
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

fn item(item: protocol::PluginItem) -> Item {
    let payload = item
        .actions
        .iter()
        .find_map(|action| match &action.effect {
            protocol::Effect::Copy { text } => Some(Payload::text(text.clone())),
            _ => None,
        })
        .unwrap_or_default();
    let actions: Vec<_> = item
        .actions
        .into_iter()
        .map(|action| {
            let run = match action.effect {
                protocol::Effect::Copy { text } => Action::Copy(text),
                protocol::Effect::OpenUrl { url } => Action::OpenUrl(url),
            };
            ItemAction::new(&action.id, &action.title, run)
        })
        .collect();
    Item {
        id: item.id,
        payload,
        view: item.view,
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
        let correlation = (self.requests.fetch_add(1, Ordering::Relaxed) + 1).to_string();
        let wire = protocol::Request {
            version: protocol::VERSION,
            request_id: correlation.clone(),
            generation: correlation,
            command: self.extension.id.clone(),
            query: request.query.clone(),
            limit: request.limit.min(protocol::MAX_RESULTS as usize) as u32,
            state: None,
        };
        let control = QueryControl {
            deadline: Instant::now() + QUERY_DEADLINE,
            is_cancelled: &|| request.is_cancelled(),
        };
        let items = match self.extension.module.query_controlled(&wire, &control) {
            Ok(response) => match response.outcome {
                protocol::Outcome::Results { items, .. } => items.into_iter().map(item).collect(),
                protocol::Outcome::Error { message, .. } => vec![self.problem(message)],
            },
            Err(HostError::Cancelled) => return Ok(()),
            Err(error) => {
                crate::log::warn(&format!("extension {}: {error}", self.extension.id));
                vec![self.problem(capitalized(&error.to_string()))]
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
    use crate::extensions::tests::{extension, module};

    fn search(provider: &ExtensionProvider, query: &str) -> Vec<Item> {
        let mut rows = Vec::new();
        provider
            .search(&Request::detached(query, 10), &mut |batch| rows.extend(batch))
            .unwrap();
        rows
    }

    #[test]
    fn guest_views_become_rows_with_only_copy_and_web_actions() {
        let response = serde_json::json!({
            "version": 1, "request_id": "1", "generation": "1",
            "outcome": {"kind": "results", "state": null, "items": [
                {"id": "count", "view": {"kind": "pair", "source": "two words", "result": "2 words"},
                 "actions": [
                    {"id": "copy", "title": "Copy", "effect": {"kind": "copy", "text": "2"}},
                    {"id": "web", "title": "Search", "effect": {"kind": "open_url", "url": "https://example.com/?q=2"}}
                 ]},
                {"id": "detail", "view": {"kind": "detail", "title": "Details", "body": "Body"}, "actions": []}
            ]}
        });
        let provider = ExtensionProvider::new(Arc::new(extension(
            "Word Count",
            "wc",
            module(&response.to_string()),
        )));
        let rows = search(&provider, "two words");
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0].view,
            View::Pair {
                source: "two words".into(),
                result: "2 words".into()
            }
        );
        assert_eq!(rows[0].primary.as_deref(), Some("copy"));
        assert_eq!(rows[0].action("copy"), Some(&Action::Copy("2".into())));
        assert_eq!(rows[0].payload, Payload::text("2"));
        assert_eq!(
            rows[0].action("web"),
            Some(&Action::OpenUrl("https://example.com/?q=2".into()))
        );
        assert!(!rows[0].record_usage);
        assert_eq!(rows[1].primary, None);
    }

    #[test]
    fn traps_and_invalid_responses_become_a_visible_error_row() {
        let trapping = dupin_plugin_runtime::GuestModule::load(
            &wat::parse_str(
                r#"(module
                (memory (export "memory") 8)
                (global $next (mut i32) (i32.const 1024))
                (func (export "dupin_abi_version") (result i32) i32.const 1)
                (func (export "dupin_alloc") (param i32) (result i32)
                    global.get $next global.get $next local.get 0 i32.add global.set $next)
                (func (export "dupin_query") (param i32 i32 i32 i32) (result i32) unreachable))"#,
            )
            .unwrap(),
        )
        .unwrap();
        let looping = dupin_plugin_runtime::GuestModule::load(
            &wat::parse_str(
                r#"(module
                (memory (export "memory") 8)
                (global $next (mut i32) (i32.const 1024))
                (func (export "dupin_abi_version") (result i32) i32.const 1)
                (func (export "dupin_alloc") (param i32) (result i32)
                    global.get $next global.get $next local.get 0 i32.add global.set $next)
                (func (export "dupin_query") (param i32 i32 i32 i32) (result i32) (loop br 0) i32.const 0))"#,
            )
            .unwrap(),
        )
        .unwrap();
        let file_url = serde_json::json!({
            "version": 1, "request_id": "1", "generation": "1",
            "outcome": {"kind": "results", "state": null, "items": [
                {"id": "x", "view": {"kind": "text", "title": "x", "subtitle": ""},
                 "actions": [{"id": "open", "title": "Open", "effect": {"kind": "open_url", "url": "file:///"}}]}
            ]}
        });
        let guest_error = serde_json::json!({
            "version": 1, "request_id": "1", "generation": "1",
            "outcome": {"kind": "error", "code": "invalid_query", "message": "Type some words"}
        });
        for (guest, subtitle) in [
            (trapping, "Plugin execution trapped"),
            (
                module(&file_url.to_string()),
                "Plugin protocol: invalid field: effect.url",
            ),
            (module(&guest_error.to_string()), "Type some words"),
        ] {
            let provider = ExtensionProvider::new(Arc::new(extension("Word Count", "wc", guest)));
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
        // Fuel or the deadline ends it first, depending on how fast this build runs.
        let provider = ExtensionProvider::new(Arc::new(extension("Word Count", "wc", looping)));
        let rows = search(&provider, "query");
        assert_eq!(rows[0].display_title(), "Word Count failed");
    }
}
