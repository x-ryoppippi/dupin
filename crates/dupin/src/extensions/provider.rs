use std::sync::Arc;
use std::time::{Duration, Instant};

use dupin_core::{
    Action, ExtensionRequest, Item, ItemAction, Page, Payload, Provider, ProviderError, Request, View,
};
use dupin_extension_bridge::{Bridge, ConsentReason, ConsentRequest, Trigger};
use dupin_extension_host::{NoServices, Services};
use dupin_js_runtime::RunError;
use dupin_plugin_protocol as protocol;

use super::Extension;

/// Shorter than the command's session deadline, so a slow guest ends with its own error row
/// rather than the generic expiry.
pub const QUERY_DEADLINE: Duration = Duration::from_secs(2);
/// An action the user chose may wait on the network or a program for longer than typing may.
pub const ACTION_DEADLINE: Duration = Duration::from_secs(30);

pub struct ExtensionProvider {
    extension: Arc<Extension>,
    bridge: Option<Bridge>,
}

impl ExtensionProvider {
    /// A provider whose guest reaches no host service.
    #[cfg(test)]
    pub fn new(extension: Arc<Extension>) -> Self {
        Self {
            extension,
            bridge: None,
        }
    }

    /// A provider whose guest's requests go through `bridge`, as typing in the launcher.
    pub fn with_bridge(extension: Arc<Extension>, bridge: Bridge) -> Self {
        Self {
            extension,
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

fn item(extension: &str, row: protocol::PluginItem) -> Item {
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
                protocol::Effect::Run { action: name, data } => {
                    let run = Action::Extension {
                        extension: extension.to_owned(),
                        request: ExtensionRequest::Run { action: name, data },
                    };
                    return ItemAction::new(&action.id, &action.title, run).keep_open();
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
        let access = self
            .bridge
            .as_ref()
            .map(|bridge| (bridge, &self.extension.package));
        let (services, generation): (Arc<dyn Services>, _) = match access {
            Some((bridge, id)) => {
                let call = bridge.call(id, Trigger::Query);
                (call.services, call.generation)
            }
            None => (Arc::new(NoServices), None),
        };
        let answer = self.extension.query(
            &request.query,
            request.page.as_ref(),
            request.limit,
            Instant::now() + QUERY_DEADLINE,
            move || cancellation.is_cancelled(),
            &services,
        );
        if let Some((bridge, id)) = access
            && !bridge.is_current(id, generation)
        {
            emit(vec![self.problem(
                "Its access changed while it ran, so its answer was dropped. Type again to retry.".into(),
            )]);
            return Ok(());
        }
        let mut items: Vec<Item> = match answer {
            Ok(rows) => rows
                .into_iter()
                .map(|row| item(&self.extension.id, row))
                .collect(),
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
        if let Some((bridge, id)) = access {
            let mut asking: Vec<_> = bridge
                .pending(Some(id))
                .iter()
                .map(|request| consent_row(&self.extension, request))
                .collect();
            if bridge.asked_while_off(id) {
                asking.insert(0, turn_on_row(&self.extension));
            }
            items.splice(0..0, asking);
        }
        emit(items);
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
    notice_row(format!("consent:{key}"), title, subtitle, actions)
}

fn turn_on_row(extension: &Extension) -> Item {
    notice_row(
        "consent:turn-on".into(),
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

    fn row(value: serde_json::Value) -> Item {
        item("word-count", serde_json::from_value(value).unwrap())
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
        let (revision_on_disk, _) = super::super::declaration_of(folder.path()).unwrap();
        assert_eq!(revision_on_disk, extension.revision);
        (
            folder,
            ExtensionProvider::with_bridge(extension, bridge.clone()),
            bridge,
        )
    }

    #[test]
    fn a_request_while_off_offers_to_turn_the_extension_on() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let (_folder, provider, bridge) = declaring(
            r#"{"storage": {"quotaBytes": 1024}}"#,
            "import { storage } from 'dupin'; export async function search() { try { await storage.keys(); } catch (error) { return [{ title: error.code }]; } }",
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
        bridge.set_enabled(&id, true).unwrap();
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
        bridge
            .allow(&id, &request.revision, &request.scope, None)
            .unwrap();
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
        assert_eq!(rows[0].primary.as_deref(), Some("run-0"));
        assert_eq!(
            rows[0].action("run-0"),
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
