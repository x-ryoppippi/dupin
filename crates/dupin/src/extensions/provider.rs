use std::sync::Arc;
use std::time::{Duration, Instant};

use dupin_core::catalogue::Entry;
use dupin_core::{
    Action, Batch, Detail, ExtensionRun, Icon, Item, ItemAction, Page, Payload, Provider, ProviderError,
    Request, View,
};
use dupin_js_runtime::RunError;
use dupin_js_runtime::row::{Row, RowEffect};
use dupin_plugin_protocol as protocol;

use super::{Answer, Extension};

/// Shorter than the command's session deadline, so a slow guest ends with its own error row
/// rather than the generic expiry.
pub const QUERY_DEADLINE: Duration = Duration::from_secs(2);

/// Ranks what `search` computed for the query above the catalogue's matches, in its own order,
/// while every row still carries a score, so usage ranking applies to the catalogue's rows.
const COMPUTED_SCORE: i32 = 1_000_000;

/// Which search an extension's provider answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Its own keyword: `search` and the catalogue.
    Keyword,
    /// The main search, from the catalogue alone, so no keystroke reaches the package.
    DefaultSearch,
}

pub struct ExtensionProvider {
    extension: Arc<Extension>,
    role: Role,
}

impl ExtensionProvider {
    #[cfg(test)]
    pub fn new(extension: Arc<Extension>) -> Self {
        Self::with_role(extension, Role::Keyword)
    }

    pub fn with_role(extension: Arc<Extension>, role: Role) -> Self {
        Self { extension, role }
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

/// The rows `extension` answers `query` with under its keyword: what `search` computes for it,
/// then the catalogue's matches, in batches by section.
pub fn answer(
    extension: &Extension,
    query: &str,
    page: Option<&Page>,
    limit: usize,
    deadline: Instant,
    is_cancelled: impl Fn() -> bool + 'static,
) -> Result<Vec<Batch>, RunError> {
    let exports = extension.check(deadline)?;
    let mut batches = Vec::new();
    // Like any keyword command, `search` runs once there is something to search for, except in
    // a view, whose filter starts empty.
    if exports.search && (page.is_some() || !query.trim().is_empty()) {
        match extension.query(query, page, limit, deadline, is_cancelled)? {
            Answer::Wasm(outcome) => batches.push(Batch::from(wasm_items(outcome)?)),
            Answer::Rows(rows) => {
                let scored = exports.items;
                let rows = rows.into_iter().enumerate().map(|(index, row)| {
                    let (mut item, detail, section) = row_item(extension, row, false);
                    if scored {
                        item.score = Some(COMPUTED_SCORE - index as i32);
                    }
                    (item, detail, section)
                });
                batches.extend(sectioned(rows));
            }
        }
    }
    if exports.items && page.is_none() {
        let left = limit.saturating_sub(batches.iter().map(|batch| batch.items.len()).sum());
        batches.extend(extension.catalogue(query, left)?);
    }
    Ok(batches)
}

/// The catalogue's matches for the main search. The package never sees the query.
pub fn default_answer(extension: &Extension, query: &str, limit: usize) -> Result<Vec<Batch>, RunError> {
    if query.trim().is_empty() || !extension.check(Instant::now() + QUERY_DEADLINE)?.items {
        return Ok(Vec::new());
    }
    extension.catalogue(query, limit)
}

/// Consecutive rows of one section in one batch, each with its detail shown while selected.
fn sectioned(rows: impl Iterator<Item = (Item, Option<Arc<Detail>>, Option<String>)>) -> Vec<Batch> {
    let mut batches: Vec<Batch> = Vec::new();
    for (item, detail, section) in rows {
        let batch = match batches.last_mut() {
            Some(batch) if batch.section == section => batch,
            _ => {
                batches.push(Batch {
                    section,
                    details_when_selected: true,
                    ..Batch::default()
                });
                batches.last_mut().expect("just pushed")
            }
        };
        if let Some(detail) = detail {
            batch.details.insert(item.id.clone(), detail);
        }
        batch.items.push(item);
    }
    batches
}

/// A catalogue row as the launcher holds it. Its ID carries the extension's, since usage
/// history keys rows by ID across every command.
pub(super) fn entry(extension: &Extension, mut row: Row) -> Entry {
    let keywords = std::mem::take(&mut row.keywords);
    let (item, detail, section) = row_item(extension, row, true);
    Entry {
        item,
        keywords,
        section: section.map(Into::into),
        detail,
    }
}

fn row_item(extension: &Extension, row: Row, catalogue: bool) -> (Item, Option<Arc<Detail>>, Option<String>) {
    let Row {
        id,
        title,
        subtitle,
        keywords: _,
        icon,
        section,
        detail,
        actions: offered,
    } = row;
    let detail = detail.map(|detail| {
        let mut shown = Detail::new(title.clone(), &detail.markdown);
        for entry in detail.metadata {
            shown = shown.with_metadata(entry.label, entry.text);
        }
        Arc::new(shown)
    });
    let payload = offered
        .iter()
        .find_map(|action| match &action.effect {
            RowEffect::Copy { text } => Some(Payload::text(text.clone())),
            _ => None,
        })
        .unwrap_or_default();
    let (mut copy, mut open) = (false, false);
    let actions: Vec<ItemAction> = offered
        .into_iter()
        .enumerate()
        .map(|(index, action)| {
            let name = |taken: &mut bool, first: &str| {
                if std::mem::replace(taken, true) {
                    format!("action-{index}")
                } else {
                    first.to_owned()
                }
            };
            match action.effect {
                RowEffect::Copy { text } => {
                    ItemAction::new(&name(&mut copy, "copy"), &action.title, Action::Copy(text))
                }
                RowEffect::OpenUrl { url } => {
                    ItemAction::new(&name(&mut open, "open"), &action.title, Action::OpenUrl(url))
                }
                RowEffect::Run { handler, data } => ItemAction::new(
                    &format!("action-{index}"),
                    &action.title,
                    Action::Extension(ExtensionRun {
                        extension: extension.id.clone(),
                        handler,
                        data,
                    }),
                )
                .keep_open(),
            }
        })
        .collect();
    let item = Item {
        id: if catalogue {
            format!("extension:{}/{id}", extension.id)
        } else {
            id
        },
        view: View::Text { title, subtitle },
        payload,
        record_usage: catalogue,
        icon: icon.map(|icon| Icon::Image(extension.root.join(icon))),
        matched: Vec::new(),
        completion: None,
        score: None,
        primary: actions.first().map(|action| action.id.clone()),
        alternate: None,
        actions,
    };
    (item, detail, section)
}

fn wasm_items(outcome: protocol::Outcome) -> Result<Vec<Item>, RunError> {
    Ok(match outcome {
        protocol::Outcome::Results { items, .. } => items
            .into_iter()
            .map(|row| item(row.id, row.view, row.actions))
            .collect(),
        protocol::Outcome::Detail {
            title,
            body,
            actions: offered,
        } => vec![item("detail".into(), View::Detail { title, body }, offered)],
        protocol::Outcome::Error { message, .. } => {
            return Err(RunError::Failed(dupin_js_runtime::Diagnostic::new(
                "guest-error",
                message,
            )));
        }
    })
}

fn item(id: String, view: View, offered: Vec<protocol::PluginAction>) -> Item {
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
        self.search_batches(request, &mut |batch| emit(batch.items))
    }

    fn search_batches(&self, request: &Request, emit: &mut dyn FnMut(Batch)) -> Result<(), ProviderError> {
        if request.limit == 0 {
            return Ok(());
        }
        let answer = match self.role {
            Role::Keyword => {
                let cancellation = request.cancellation();
                answer(
                    &self.extension,
                    &request.query,
                    request.page.as_ref(),
                    request.limit,
                    Instant::now() + QUERY_DEADLINE,
                    move || cancellation.is_cancelled(),
                )
            }
            Role::DefaultSearch => default_answer(&self.extension, &request.query, request.limit),
        };
        if request.is_cancelled() {
            return Ok(());
        }
        match answer {
            Ok(batches) => batches.into_iter().for_each(emit),
            Err(RunError::Cancelled) => {}
            Err(RunError::Failed(diagnostic)) => {
                crate::log::warn(&format!("extension {}: {diagnostic}", self.extension.id));
                // The main search shows everyone's results; a failing extension only logs.
                if self.role == Role::Keyword {
                    // A located problem starts with its file name, which keeps its case.
                    let subtitle = if diagnostic.file.is_some() {
                        diagnostic.to_string()
                    } else {
                        capitalized(&diagnostic.message)
                    };
                    emit(Batch::from(vec![self.problem(subtitle)]));
                }
            }
        }
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

        let root = provider.extension.wire("q", None, 10);
        assert_eq!((root.view, root.context), (None, None));
        let wire = provider.extension.wire("filter", Some(&page()), 10);
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
    fn the_javascript_word_counter_answers_like_the_wasm_one() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions/word-count-js");
        let extension = super::super::load_folder(&root).unwrap();
        assert_eq!(
            (extension.runtime(), extension.keyword.as_str()),
            ("javascript", "wc")
        );
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
