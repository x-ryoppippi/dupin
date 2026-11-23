use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use dupin_core::{Action, Item, ItemAction, Page, Provider, ProviderError, Request, View};
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

fn actions(actions: Vec<protocol::PluginAction>) -> Vec<ItemAction> {
    actions
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
        .collect()
}

fn item(id: String, view: View, actions: Vec<ItemAction>) -> Item {
    Item {
        id,
        view,
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

impl ExtensionProvider {
    fn wire(&self, request: &Request) -> protocol::Request {
        let correlation = (self.requests.fetch_add(1, Ordering::Relaxed) + 1).to_string();
        let page = request.page.as_ref();
        protocol::Request {
            version: protocol::VERSION,
            request_id: correlation.clone(),
            generation: correlation,
            command: self.extension.id.clone(),
            query: request.query.clone(),
            limit: request.limit.min(protocol::MAX_RESULTS as usize) as u32,
            state: None,
            view: page.map(|page| page.view.clone()),
            context: page.and_then(|page| page.context.clone()),
        }
    }
}

impl Provider for ExtensionProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        if request.limit == 0 {
            return Ok(());
        }
        let wire = self.wire(request);
        let control = QueryControl {
            deadline: Instant::now() + QUERY_DEADLINE,
            is_cancelled: &|| request.is_cancelled(),
        };
        let items = match self.extension.module.query_controlled(&wire, &control) {
            Ok(response) => match response.outcome {
                protocol::Outcome::Results { items, .. } => items
                    .into_iter()
                    .map(|row| item(row.id, row.view, actions(row.actions)))
                    .collect(),
                protocol::Outcome::Detail {
                    title,
                    body,
                    actions: offered,
                } => vec![item(
                    "detail".into(),
                    View::Detail { title, body },
                    actions(offered),
                )],
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
        assert_eq!(
            rows[0].action("web"),
            Some(&Action::OpenUrl("https://example.com/?q=2".into()))
        );
        assert!(!rows[0].record_usage);
        assert_eq!(rows[1].primary, None);
    }

    fn page() -> Page {
        Page {
            view: "stats".into(),
            title: "Statistics".into(),
            context: Some(serde_json::json!({"text": "two words"})),
        }
    }

    #[test]
    fn push_effects_become_push_actions_and_views_reach_the_guest() {
        let response = serde_json::json!({
            "version": 1, "request_id": "1", "generation": "1",
            "outcome": {"kind": "results", "state": null, "items": [
                {"id": "count", "view": {"kind": "text", "title": "2 words", "subtitle": ""},
                 "actions": [{"id": "stats", "title": "Statistics", "effect":
                    {"kind": "push", "view": "stats", "title": "Statistics", "context": {"text": "two words"}}}]}
            ]}
        });
        let provider = ExtensionProvider::new(Arc::new(extension(
            "Word Count",
            "wc",
            module(&response.to_string()),
        )));
        let rows = search(&provider, "two words");
        assert_eq!(rows[0].action("stats"), Some(&Action::Push(page())));
        assert_eq!(rows[0].primary.as_deref(), Some("stats"));

        let root = provider.wire(&Request::detached("q", 10));
        assert_eq!((root.view, root.context), (None, None));
        let wire = provider.wire(&Request::detached_page("filter", page(), 10));
        assert_eq!(wire.query, "filter");
        assert_eq!(wire.view.as_deref(), Some("stats"));
        assert_eq!(wire.context, page().context);
        assert!(wire.encode().is_ok());
    }

    #[test]
    fn a_detail_answer_to_a_view_is_one_detail_row_and_is_refused_outside_one() {
        let response = serde_json::json!({
            "version": 1, "request_id": "1", "generation": "1",
            "outcome": {"kind": "detail", "title": "Statistics", "body": "2 words\n9 characters",
                "actions": [{"id": "copy", "title": "Copy", "effect": {"kind": "copy", "text": "2"}}]}
        });
        let provider = ExtensionProvider::new(Arc::new(extension(
            "Word Count",
            "wc",
            module(&response.to_string()),
        )));
        let mut rows = Vec::new();
        provider
            .search(&Request::detached_page("", page(), 10), &mut |batch| {
                rows.extend(batch)
            })
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].view,
            View::Detail {
                title: "Statistics".into(),
                body: "2 words\n9 characters".into()
            }
        );
        assert_eq!(rows[0].action("copy"), Some(&Action::Copy("2".into())));
        let provider = ExtensionProvider::new(Arc::new(extension(
            "Word Count",
            "wc",
            module(&response.to_string()),
        )));
        let rows = search(&provider, "two words");
        assert_eq!(
            rows[0].view,
            View::Text {
                title: "Word Count failed".into(),
                subtitle: "Plugin protocol: invalid field: outcome".into()
            }
        );
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
