use std::sync::Arc;
use std::time::{Duration, Instant};

use dupin_core::catalogue::Entry;
use dupin_core::{
    Action, Batch, Detail, ExtensionRequest, Icon, Item, ItemAction, Page, Payload, Provider, ProviderError,
    Request, View,
};
use dupin_extension_bridge::{Bridge, Call, ConsentReason, ConsentRequest, PackageId, Trigger};
use dupin_js_runtime::RunError;
use dupin_js_runtime::row::{Row, RowEffect};

use super::Extension;
use super::catalogue::Convert;

/// Shorter than the command's session deadline, so a slow guest ends with its own error row
/// rather than the generic expiry.
pub const QUERY_DEADLINE: Duration = Duration::from_secs(2);
/// An action the user chose may wait on the network or a program for longer than typing may.
pub const ACTION_DEADLINE: Duration = Duration::from_secs(30);

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
    bridge: Option<Bridge>,
}

impl ExtensionProvider {
    /// A provider for the keyword whose guest reaches no host service.
    #[cfg(test)]
    pub fn new(extension: Arc<Extension>) -> Self {
        Self::with_role(extension, Role::Keyword)
    }

    /// A provider whose guest reaches no host service.
    #[cfg(test)]
    pub fn with_role(extension: Arc<Extension>, role: Role) -> Self {
        Self {
            extension,
            role,
            bridge: None,
        }
    }

    /// A provider whose guest's requests go through `bridge`, as typing in the launcher.
    pub fn with_bridge(extension: Arc<Extension>, role: Role, bridge: Bridge) -> Self {
        Self {
            extension,
            role,
            bridge: Some(bridge),
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

/// The rows `extension` answers `query` with under its keyword: what `search` computes for it,
/// then the catalogue's matches, in batches by section. Both answer their host requests through
/// `call`.
pub fn answer(
    extension: &Extension,
    query: &str,
    page: Option<&Page>,
    limit: usize,
    deadline: Instant,
    is_cancelled: impl Fn() -> bool + 'static,
    call: &Call,
) -> Result<Vec<Batch>, RunError> {
    if page.is_some() {
        return Err(super::views_unsupported());
    }
    let exports = extension.check(deadline)?;
    let mut batches = Vec::new();
    // Like any keyword command, `search` runs once there is something to search for.
    if exports.search && !query.trim().is_empty() {
        let scored = exports.items;
        let rows = extension.query(query, None, limit, deadline, is_cancelled, &call.services)?;
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
        batches.extend(extension.catalogue(query, left, deadline, call, Role::Keyword)?);
        let left = limit.saturating_sub(batches.iter().map(|batch| batch.items.len()).sum());
        let searches: Vec<Item> = extension
            .web_searches(deadline, call, Role::Keyword)?
            .iter()
            .filter_map(|(item, search)| crate::web::item(item, search, query.trim(), false))
            .take(left)
            .collect();
        if !searches.is_empty() {
            batches.push(Batch::from(searches));
        }
    }
    Ok(batches)
}

/// The catalogue's matches for the main search. The package never sees the query, but loading
/// its catalogue reaches host services through `call` as it does under the keyword.
pub fn default_answer(
    extension: &Extension,
    query: &str,
    limit: usize,
    deadline: Instant,
    call: &Call,
) -> Result<Vec<Batch>, RunError> {
    if query.trim().is_empty() || !extension.check(deadline)?.items {
        return Ok(Vec::new());
    }
    extension.catalogue(query, limit, deadline, call, Role::DefaultSearch)
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
        let web_search = row.web_search.take();
        let (item, detail, section) = row_item(self, row, true);
        Entry {
            item,
            keywords,
            section: section.map(Into::into),
            detail,
            web_search,
        }
    }

    /// Carries the extension's ID, since usage history keys rows by ID across every command.
    fn entry_id(&self, id: &str) -> String {
        format!("extension:{}/{id}", self.id)
    }
}

pub(super) fn row_item(
    extension: &Extension,
    row: Row,
    catalogue: bool,
) -> (Item, Option<Arc<Detail>>, Option<String>) {
    let Row {
        id,
        title,
        subtitle,
        keywords: _,
        icon,
        section,
        detail,
        actions: offered,
        web_search: _,
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
                    Action::Extension {
                        extension: extension.id.clone(),
                        request: ExtensionRequest::Run {
                            action: handler,
                            data,
                        },
                    },
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
        let deadline = Instant::now() + QUERY_DEADLINE;
        let id = &self.extension.package;
        // Loading a catalogue is a query too, from either search, with no activation.
        let call = match &self.bridge {
            Some(bridge) => bridge.call(id, Trigger::Query),
            None => Call::none(),
        };
        let answer = match self.role {
            Role::Keyword => {
                let cancellation = request.cancellation();
                answer(
                    &self.extension,
                    &request.query,
                    request.page.as_ref(),
                    request.limit,
                    deadline,
                    move || cancellation.is_cancelled(),
                    &call,
                )
            }
            Role::DefaultSearch => {
                default_answer(&self.extension, &request.query, request.limit, deadline, &call)
            }
        };
        if let Some(bridge) = &self.bridge
            && !bridge.is_current(id, call.generation)
        {
            // The main search shows everyone's results; a package whose access changed only
            // drops its own.
            if self.role == Role::Keyword {
                emit(Batch::from(vec![self.problem(
                    "Its access changed while it ran, so its answer was dropped. Type again to retry.".into(),
                )]));
            }
            return Ok(());
        }
        if request.is_cancelled() || matches!(answer, Err(RunError::Cancelled)) {
            return Ok(());
        }
        // Only the keyword asks for consent; the main search never shows another package's
        // questions among its results.
        let access = match self.role {
            Role::Keyword => self.bridge.as_ref().map(|bridge| (bridge, id)),
            Role::DefaultSearch => None,
        };
        if let Some((bridge, id)) = access {
            let mut asking: Vec<_> = bridge
                .pending(Some(id))
                .iter()
                .map(|request| consent_row(&self.extension, request))
                .collect();
            if bridge.asked_while_off(id) {
                asking.insert(0, turn_on_row(&self.extension));
            }
            if !asking.is_empty() {
                emit(Batch::from(asking));
            }
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

fn extension_action(extension: &Extension, id: &str, title: &str, request: ExtensionRequest) -> ItemAction {
    ItemAction::new(
        id,
        title,
        Action::Extension {
            extension: extension.id.clone(),
            request,
        },
    )
    .consumes()
}

/// A row that asks the user to decide. It sits above the extension's results, where Enter
/// lands while typing, so Enter opens its choices instead of granting anything.
fn notice_row(id: String, title: String, subtitle: String, actions: Vec<ItemAction>) -> Item {
    Item {
        id,
        payload: Payload::default(),
        view: View::Text { title, subtitle },
        record_usage: false,
        icon: None,
        matched: Vec::new(),
        completion: None,
        score: None,
        primary: None,
        alternate: None,
        actions,
    }
}

/// What the panel shows when the extension asked for a scope the user hasn't decided on: the
/// scope as the broker describes it, and Allow and Deny. Nothing runs until the user chooses,
/// and choosing runs nothing the extension asked for before.
pub(crate) fn consent_row(extension: &Extension, request: &ConsentRequest) -> Item {
    let key = request.key().to_string();
    let title = format!("Allow {} to {}?", extension.name, request.description);
    let (subtitle, actions) = match request.reason {
        ConsentReason::OsAccess(permission) => (
            format!(
                "macOS hasn't allowed Dupin {} yet. Requesting it asks macOS once.",
                permission.name()
            ),
            vec![extension_action(
                extension,
                "request-access",
                "Request Access",
                ExtensionRequest::RequestAccess { scope: key.clone() },
            )],
        ),
        _ if request.needs_binding() => (
            format!("{key} · Choose what it may use in Settings → Extensions."),
            vec![extension_action(
                extension,
                "review",
                "Open Settings",
                ExtensionRequest::Review,
            )],
        ),
        reason => (
            if reason == ConsentReason::Changed {
                format!(
                    "{key} · An update changed what it asks for. Allowing runs nothing until you use it again."
                )
            } else {
                format!("{key} · Allowing runs nothing until you use it again.")
            },
            vec![
                extension_action(
                    extension,
                    "allow",
                    "Allow",
                    ExtensionRequest::Allow {
                        scope: key.clone(),
                        revision: request.revision.to_string(),
                    },
                ),
                extension_action(
                    extension,
                    "deny",
                    "Deny",
                    ExtensionRequest::Deny { scope: key.clone() },
                ),
            ],
        ),
    };
    let mut actions = actions;
    if !actions.iter().any(|action| action.id == "deny") {
        actions.push(extension_action(
            extension,
            "deny",
            "Deny",
            ExtensionRequest::Deny { scope: key.clone() },
        ));
    }
    notice_row(format!("{CONSENT_ROW}{key}"), title, subtitle, actions)
}

const CONSENT_ROW: &str = "consent:";
const TURN_ON_ROW: &str = "consent:turn-on";

/// Whether `row` asks the user something about `id` that the bridge no longer waits on, so the
/// panel can drop it after an answer without running the guest again.
pub(crate) fn is_answered_consent_row(row: &Item, bridge: &Bridge, id: &PackageId) -> bool {
    let Some(asked) = row.id.strip_prefix(CONSENT_ROW) else {
        return false;
    };
    if row.id == TURN_ON_ROW {
        return !bridge.asked_while_off(id);
    }
    !bridge
        .pending(Some(id))
        .iter()
        .any(|request| request.key().to_string() == asked)
}

fn turn_on_row(extension: &Extension) -> Item {
    notice_row(
        TURN_ON_ROW.into(),
        format!("Turn on {}?", extension.name),
        "It asked to use something while it's off. Turning it on lets it ask for each thing it declares."
            .into(),
        vec![
            extension_action(extension, "turn-on", "Turn On", ExtensionRequest::TurnOn),
            extension_action(extension, "review", "Open Settings", ExtensionRequest::Review),
        ],
    )
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

    /// A JavaScript extension that declares `capabilities`, behind an in-memory bridge.
    fn declaring(capabilities: &str, source: &str) -> (tempfile::TempDir, ExtensionProvider, Bridge) {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(
            folder.path().join(super::super::manifest::PACKAGE_JSON),
            format!(
                r#"{{"name": "word-count", "version": "0.1.0", "dupin": {{"title": "Word Count", "keyword": "wc", "capabilities": {capabilities}}}}}"#
            ),
        )
        .unwrap();
        std::fs::write(folder.path().join("main.js"), source).unwrap();
        let extension = super::super::load_folder(folder.path()).unwrap();
        let bridge = Bridge::new(
            dupin_extension_bridge::Broker::new(Default::default(), Default::default()),
            None,
            dupin_extension_bridge::Services::none(),
        );
        let (id, revision) = (extension.package.clone(), extension.revision.clone());
        bridge.install(&id, revision, extension.capabilities.clone());
        let (package_on_disk, revision_on_disk, _) = super::super::declaration_of(folder.path()).unwrap();
        assert_eq!(package_on_disk, extension.package);
        assert_eq!(revision_on_disk, extension.revision);
        (
            folder,
            ExtensionProvider::with_bridge(extension, Role::Keyword, bridge.clone()),
            bridge,
        )
    }

    #[test]
    fn a_request_while_off_offers_to_turn_the_extension_on() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let (_folder, provider, bridge) = declaring(
            r#"{"storage": {"quotaBytes": 1024}}"#,
            "import { storage } from 'dupin'; export async function search() { try { await storage.usage(); } catch (error) { return [{ title: error.code }]; } }",
        );
        let rows = search(&provider, "x");
        assert_eq!(rows[0].id, "consent:turn-on");
        assert_eq!(rows[0].display_title(), "Turn on Word Count?");
        assert_eq!(
            rows[0].action("turn-on"),
            Some(&Action::Extension {
                extension: "word-count".into(),
                request: ExtensionRequest::TurnOn
            })
        );
        assert_eq!(rows[1].display_title(), "disabled");

        let id = provider.extension.package.clone();
        assert!(!is_answered_consent_row(&rows[0], &bridge, &id));
        bridge.set_enabled(&id, true).unwrap();
        assert!(is_answered_consent_row(&rows[0], &bridge, &id));
        assert!(
            !is_answered_consent_row(&rows[1], &bridge, &id),
            "only questions go"
        );
        let rows = search(&provider, "x");
        assert_eq!(rows[0].id, "consent:storage");
        assert_eq!(rows[0].primary, None, "Enter shows the choices, never grants");
        assert_eq!(
            rows[0].display_title(),
            "Allow Word Count to keep up to 1024 bytes of encrypted data?"
        );
        let Some(Action::Extension {
            request: ExtensionRequest::Allow { scope, revision },
            ..
        }) = rows[0].action("allow")
        else {
            panic!("{:?}", rows[0].actions);
        };
        assert_eq!(scope, "storage");
        assert_eq!(revision, provider.extension.revision.as_str());
        assert!(rows[0].action("deny").is_some());
        assert_eq!(rows[1].display_title(), "needs-consent");

        let request = bridge.pending(Some(&id)).remove(0);
        assert!(!is_answered_consent_row(&rows[0], &bridge, &id));
        bridge
            .allow(&id, &request.revision, &request.scope, None)
            .unwrap();
        assert!(
            is_answered_consent_row(&rows[0], &bridge, &id),
            "the panel drops the answered row without searching again"
        );
        let rows = search(&provider, "x");
        assert_eq!(rows.len(), 1, "the request is answered");
        // The in-memory bridge reaches no service, so the granted request now fails there.
        assert_eq!(rows[0].display_title(), "unavailable");
    }

    #[test]
    fn rows_offering_actions_run_them_in_their_own_extension() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let (_folder, provider, _bridge) = declaring(
            "{}",
            "export const search = () => [{ title: 'Note', actions: [{ title: 'Save', run: 'save', data: { id: 7 } }] }];",
        );
        let rows = search(&provider, "x");
        assert_eq!(rows[0].primary.as_deref(), Some("action-0"));
        assert_eq!(
            rows[0].action("action-0"),
            Some(&Action::Extension {
                extension: "word-count".into(),
                request: ExtensionRequest::Run {
                    action: "save".into(),
                    data: Some(serde_json::json!({"id": 7}))
                }
            })
        );
    }
}
