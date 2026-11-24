use std::sync::Arc;
use std::time::{Duration, Instant};

use dupin_core::catalogue::Entry;
use dupin_core::{
    Action, Batch, Detail, ExtensionRun, Icon, Item, ItemAction, Page, Payload, Provider, ProviderError,
    Request, View,
};
use dupin_js_runtime::RunError;
use dupin_js_runtime::row::{Row, RowEffect};

use super::Extension;
use super::catalogue::Convert;

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
    if page.is_some() {
        return Err(super::views_unsupported());
    }
    let exports = extension.check(deadline)?;
    let mut batches = Vec::new();
    // Like any keyword command, `search` runs once there is something to search for.
    if exports.search && !query.trim().is_empty() {
        let scored = exports.items;
        let rows = extension.query(query, None, limit, deadline, is_cancelled)?;
        let rows = rows.into_iter().enumerate().map(|(index, row)| {
            let (mut item, detail, section) = row_item(extension, row, false);
            if scored {
                item.score = Some(COMPUTED_SCORE - index as i32);
            }
            (item, detail, section)
        });
        batches.extend(sectioned(rows));
    }
    if exports.items {
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

impl Convert for Extension {
    /// A catalogue row as the launcher holds it.
    fn entry(&self, mut row: Row) -> Entry {
        let keywords = std::mem::take(&mut row.keywords);
        let (item, detail, section) = row_item(self, row, true);
        Entry {
            item,
            keywords,
            section: section.map(Into::into),
            detail,
        }
    }

    /// Carries the extension's ID, since usage history keys rows by ID across every command.
    fn entry_id(&self, id: &str) -> String {
        format!("extension:{}/{id}", self.id)
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
        id: if catalogue { extension.entry_id(&id) } else { id },
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

    fn search(provider: &ExtensionProvider, query: &str) -> Vec<Item> {
        let mut rows = Vec::new();
        provider
            .search(&Request::detached(query, 10), &mut |batch| rows.extend(batch))
            .unwrap();
        rows
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
