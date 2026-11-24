//! `items()` packages through the provider: the catalogue, its sections and details, actions
//! that invalidate it, the main search and parity with a native catalogue.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dupin_core::catalogue::{Catalogue, Change, Entry};
use dupin_core::{Action, Batch, Item, ItemAction, Payload, Provider, ProviderError, Request, View};
use dupin_extension_bridge::Call;

use super::parity::{Parity, seen};
use super::{Extension, ExtensionProvider, QUERY_DEADLINE, Role, catalogue};

fn package(manifest: &str, files: &[(&str, &str)]) -> (tempfile::TempDir, Arc<Extension>) {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join(super::manifest::PACKAGE_JSON),
        format!(r#"{{"name": "marks", "version": "0.1.0", "dupin": {{"title": "Marks", "keyword": "bm"{manifest}}}}}"#),
    )
    .unwrap();
    for (name, source) in files {
        let path = folder.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    let extension = super::load_folder(folder.path()).unwrap();
    (folder, extension)
}

fn no_services() -> Arc<dyn dupin_extension_host::Services> {
    Arc::new(dupin_extension_host::NoServices)
}

fn batches(provider: &ExtensionProvider, query: &str) -> Vec<Batch> {
    let mut batches = Vec::new();
    provider
        .search_batches(&Request::detached(query, 50), &mut |batch| batches.push(batch))
        .unwrap();
    batches
}

fn titles(batches: &[Batch]) -> Vec<&str> {
    batches
        .iter()
        .flat_map(|batch| &batch.items)
        .map(Item::display_title)
        .collect()
}

const BOOKMARKS: &str = r#"
import { items as catalogue } from 'dupin';
export function items(ctx) {
  return {
    cursor: ctx.reason,
    sections: [
      { title: 'Projects', items: [
        { id: 'dupin', title: 'Dupin', subtitle: `${ctx.reason} after ${ctx.cursor}`, keywords: ['launcher'], icon: 'icons/dupin.png',
          url: 'https://github.com/x-ryoppippi/dupin', detail: { markdown: '**Fast**', metadata: [{ label: 'Stars', text: '9' }] },
          actions: [{ title: 'Forget', run: 'forget', data: { id: 'dupin' } }] },
        { id: 'rune', title: 'Rune', url: 'https://github.com/anomalyco/rune' },
      ] },
      { title: 'Reading', items: [{ id: 'rust', title: 'The Rust Book', text: 'https://doc.rust-lang.org/book/' }] },
    ],
  };
}
export const actions = { forget() { catalogue.invalidate(); } };
"#;

#[test]
fn the_catalogue_lists_its_sections_and_typing_ranks_it_natively() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let (folder, extension) = package("", &[("main.js", BOOKMARKS)]);
    let provider = ExtensionProvider::new(extension.clone());

    let listed = batches(&provider, "");
    let sections: Vec<_> = listed
        .iter()
        .map(|batch| (batch.section.as_deref(), batch.items.len()))
        .collect();
    assert_eq!(sections, [(Some("Projects"), 2), (Some("Reading"), 1)]);
    assert!(listed.iter().all(|batch| batch.details_when_selected));
    let dupin = &listed[0].items[0];
    assert_eq!(dupin.id, "extension:marks/dupin");
    assert!(dupin.record_usage);
    assert_eq!(dupin.score, None);
    assert_eq!(
        dupin.icon,
        Some(dupin_core::Icon::Image(folder.path().join("icons/dupin.png")))
    );
    assert_eq!(dupin.primary.as_deref(), Some("open"));
    assert!(matches!(
        dupin.action("action-1"),
        Some(Action::Extension {
            request: dupin_core::ExtensionRequest::Run { action, data },
            ..
        }) if action == "forget" && *data == Some(serde_json::json!({"id": "dupin"}))
    ));
    let detail = &listed[0].details[&dupin.id];
    assert_eq!(
        (detail.title.as_str(), detail.metadata[0].value.as_str()),
        ("Dupin", "9")
    );
    assert_eq!(
        listed[1].items[0].payload,
        Payload::text("https://doc.rust-lang.org/book/")
    );

    let typed = batches(&provider, "launch");
    assert_eq!(titles(&typed), ["Dupin"]);
    assert_eq!(typed[0].section, None);
    assert!(typed[0].items[0].score.is_some());
    assert_eq!(titles(&batches(&provider, "ru")), ["Rune", "The Rust Book"]);
    if let View::Text { subtitle, .. } = &typed[0].items[0].view {
        assert_eq!(subtitle, "load after null");
    }

    // The handler invalidates the catalogue; the next search asks again with the cursor.
    let acted = extension
        .act(
            "forget",
            &serde_json::json!({"id": "dupin"}),
            Instant::now() + QUERY_DEADLINE,
            &no_services(),
        )
        .unwrap();
    assert!(acted.invalidated);
    let again = batches(&provider, "dupin");
    let View::Text { subtitle, .. } = &again[0].items[0].view else {
        panic!("{again:?}");
    };
    assert_eq!(subtitle, "invalidated after load");
}

#[test]
fn search_answers_come_first_and_the_catalogue_fills_the_rest() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let (_folder, extension) = package(
        "",
        &[(
            "main.js",
            r#"export function items() { return [{ id: 'a', title: 'Alpha docs' }, { id: 'b', title: 'Beta docs' }]; }
               export function search(query) { return [{ title: `Search the web for ${query}`, url: 'https://example.com/' }]; }"#,
        )],
    );
    let provider = ExtensionProvider::new(extension);
    let rows: Vec<_> = batches(&provider, "docs")
        .into_iter()
        .flat_map(|batch| batch.items)
        .collect();
    let titles: Vec<_> = rows.iter().map(Item::display_title).collect();
    // The shorter title wins an equal match, as it does among apps.
    assert_eq!(titles, ["Search the web for docs", "Beta docs", "Alpha docs"]);
    assert!(rows[0].score > rows[1].score);
    assert!(!rows[0].record_usage && rows[1].record_usage);
    // An empty query asks `search` nothing and lists the catalogue.
    assert_eq!(titles_of(&provider, ""), ["Alpha docs", "Beta docs"]);
}

fn titles_of(provider: &ExtensionProvider, query: &str) -> Vec<String> {
    titles(&batches(provider, query))
        .into_iter()
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_main_search_matches_only_the_catalogue_and_hides_failures() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let (_folder, extension) = package(
        r#", "defaultSearch": true"#,
        &[(
            "main.js",
            r#"export function items() { return [{ id: 'a', title: 'Alpha docs' }]; }
               export function search() { throw new Error('the main search never calls search'); }"#,
        )],
    );
    assert!(extension.default_search);
    let provider = ExtensionProvider::with_role(extension, Role::DefaultSearch);
    assert!(batches(&provider, "  ").is_empty());
    assert_eq!(titles_of(&provider, "alp"), ["Alpha docs"]);

    let (_folder, broken) = package(
        r#", "defaultSearch": true"#,
        &[("main.js", "export function items() { throw new Error('boom'); }")],
    );
    assert!(
        batches(
            &ExtensionProvider::with_role(broken.clone(), Role::DefaultSearch),
            "x"
        )
        .is_empty()
    );
    assert_eq!(titles_of(&ExtensionProvider::new(broken), "x"), ["Marks failed"]);
}

#[test]
fn a_catalogue_change_the_host_rejects_names_the_package() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let (_folder, extension) = package(
        "",
        &[(
            "main.js",
            "export function items() { return { type: 'update', replace: [{ id: 'ghost', title: 'Ghost' }] }; }",
        )],
    );
    let rows = titles_of(&ExtensionProvider::new(extension.clone()), "");
    assert_eq!(rows, ["Marks failed"]);
    let error = extension
        .catalogue(
            "",
            10,
            Instant::now() + QUERY_DEADLINE,
            &Call::none(),
            Role::Keyword,
        )
        .unwrap_err();
    assert!(
        matches!(&error, dupin_js_runtime::RunError::Failed(diagnostic)
            if diagnostic.code == "unknown-id" && diagnostic.file.as_deref() == Some("main.js")),
        "{error:?}"
    );
}

#[test]
fn an_update_removes_rows_from_the_keyword_and_the_main_search() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let (_folder, extension) = package(
        r#", "defaultSearch": true"#,
        &[(
            "main.js",
            r#"import { items as catalogue } from 'dupin';
               export function items(ctx) {
                 if (ctx.reason === 'load') {
                   return { cursor: '1', items: [{ id: 'clip-1', title: 'Secret token', text: 'hunter2' }, { id: 'clip-2', title: 'Shopping list', text: 'eggs' }] };
                 }
                 return { type: 'update', remove: ['clip-1'], cursor: '2' };
               }
               export const actions = { forget() { catalogue.invalidate(); } };"#,
        )],
    );
    let keyword = ExtensionProvider::new(extension.clone());
    let main = ExtensionProvider::with_role(extension.clone(), Role::DefaultSearch);
    assert_eq!(titles_of(&keyword, ""), ["Secret token", "Shopping list"]);
    assert_eq!(titles_of(&main, "secret"), ["Secret token"]);

    let acted = extension
        .act(
            "forget",
            &serde_json::Value::Null,
            Instant::now() + QUERY_DEADLINE,
            &no_services(),
        )
        .unwrap();
    assert!(acted.invalidated);
    assert_eq!(titles_of(&keyword, ""), ["Shopping list"]);
    assert!(titles_of(&main, "secret").is_empty());
    assert_eq!(extension.catalogue.len(), 1);
}

#[test]
fn an_event_that_invalidates_the_catalogue_updates_it_incrementally() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let (_folder, extension) = package(
        "",
        &[(
            "main.js",
            r#"import { items as catalogue } from 'dupin';
               export function items(ctx) {
                 if (ctx.reason === 'load') return { cursor: '1', items: [{ id: 'clip-1', title: 'First copy' }] };
                 return { type: 'update', append: [{ id: `clip-${Number(ctx.cursor) + 1}`, title: `${ctx.reason} after ${ctx.cursor}` }], cursor: String(Number(ctx.cursor) + 1) };
               }
               export const events = { clipboard(change) { if (change.type === 'text') catalogue.invalidate(); } };"#,
        )],
    );
    let keyword = ExtensionProvider::new(extension.clone());
    assert_eq!(titles_of(&keyword, ""), ["First copy"]);
    let copied = serde_json::json!({"type": "text", "text": "second"});
    let handled = extension
        .event(
            "clipboard",
            &copied,
            Instant::now() + QUERY_DEADLINE,
            &no_services(),
        )
        .unwrap();
    assert!(handled.invalidated);
    assert_eq!(titles_of(&keyword, ""), ["First copy", "invalidated after 1"]);
    let ignored = extension
        .event(
            "clipboard",
            &serde_json::json!({"type": "image"}),
            Instant::now() + QUERY_DEADLINE,
            &no_services(),
        )
        .unwrap();
    assert!(!ignored.invalidated);
    assert_eq!(titles_of(&keyword, ""), ["First copy", "invalidated after 1"]);
}

/// A provider for `extension` whose guest's requests go through an in-memory bridge that holds
/// the package's declaration, with no grant yet.
fn bridged(extension: &Arc<Extension>, role: Role) -> (ExtensionProvider, dupin_extension_bridge::Bridge) {
    let bridge = dupin_extension_bridge::Bridge::new(
        dupin_extension_bridge::Broker::new(Default::default(), Default::default()),
        None,
        dupin_extension_bridge::Services::none(),
    );
    bridge.install(
        &extension.package,
        extension.revision.clone(),
        extension.capabilities.clone(),
    );
    (
        ExtensionProvider::with_bridge(extension.clone(), role, bridge.clone()),
        bridge,
    )
}

#[test]
fn a_catalogue_asks_the_broker_as_a_search_does_and_never_with_an_activation() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let (_folder, extension) = package(
        r#", "defaultSearch": true, "capabilities": {"storage": {"quotaBytes": 1024}, "clipboard": ["read"]}"#,
        &[(
            "main.js",
            r#"import { storage, clipboard } from 'dupin';
               const code = (promise) => promise.then(() => 'ok', (error) => error.code);
               export async function items() {
                 const stored = await code(storage.usage());
                 if (stored === 'needs-consent') throw new Error('storage needs consent');
                 return [
                   { id: 'storage', title: `storage ${stored}` },
                   { id: 'clipboard', title: `clipboard ${await code(clipboard.readText())}` },
                 ];
               }"#,
        )],
    );
    let (keyword, bridge) = bridged(&extension, Role::Keyword);
    let main = ExtensionProvider::with_bridge(extension.clone(), Role::DefaultSearch, bridge.clone());
    let id = &extension.package;
    bridge.set_enabled(id, true).unwrap();

    let rows = titles_of(&keyword, "");
    assert_eq!(
        rows[0], "Allow Marks to keep up to 1024 bytes of encrypted data?",
        "{rows:?}"
    );
    assert_eq!(rows[1], "Marks failed");
    assert!(
        titles_of(&main, "storage").is_empty(),
        "the main search neither lists the failure nor asks for consent"
    );

    let request = &bridge.pending(Some(id))[0];
    bridge.allow(id, &request.revision, &request.scope, None).unwrap();
    // The app forgets the catalogue when grants change; the bridge here has no app to tell.
    extension.forget_catalogue();
    assert_eq!(
        titles_of(&keyword, ""),
        // The in-memory bridge has no storage service, so the granted request reaches it and
        // fails there; reading the clipboard needs an action you chose.
        ["storage unavailable", "clipboard needs-activation"]
    );
    assert_eq!(titles_of(&main, "clip"), ["clipboard needs-activation"]);

    let key = request.key();
    bridge.revoke(id, &key).unwrap();
    assert_eq!(
        titles_of(&keyword, ""),
        ["storage revoked", "clipboard needs-activation"],
        "revoking drops the rows read under the grant and asks for the catalogue again"
    );
}

#[test]
fn every_part_of_a_catalogue_shares_one_deadline() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let (_folder, extension) = package(
        "",
        &[(
            "main.js",
            r#"export function items(ctx) {
                 const end = Date.now() + 150;
                 while (Date.now() < end) {}
                 const part = Number(ctx.cursor ?? 0) + 1;
                 return { more: part < 10, cursor: String(part), items: [{ id: `part-${part}`, title: `Part ${part}` }] };
               }"#,
        )],
    );
    // Linked first, so the deadline below only has the parts to cover.
    extension.check(Instant::now() + QUERY_DEADLINE).unwrap();
    // Short, since the busy parts hold the extension host every other test shares.
    let deadline = Duration::from_millis(400);
    let started = Instant::now();
    let error = extension
        .catalogue("", 10, started + deadline, &Call::none(), Role::Keyword)
        .unwrap_err();
    let elapsed = started.elapsed();
    assert!(
        matches!(&error, dupin_js_runtime::RunError::Failed(diagnostic) if diagnostic.code == "deadline"),
        "{error:?}"
    );
    assert!(elapsed < deadline + Duration::from_millis(300), "{elapsed:?}");
    assert_eq!(extension.catalogue.len(), 0);
}

/// A native provider over a catalogue, as a built-in port's baseline would be.
struct NativeCatalogue(Catalogue);

impl Provider for NativeCatalogue {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        self.search_batches(request, &mut |batch| emit(batch.items))
    }

    fn search_batches(&self, request: &Request, emit: &mut dyn FnMut(Batch)) -> Result<(), ProviderError> {
        self.0
            .search(&request.query, request.limit, 0)
            .into_iter()
            .for_each(emit);
        Ok(())
    }
}

#[test]
fn a_port_answers_like_the_native_catalogue_it_replaces() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let rows = r#"[
  { "id": "dupin", "title": "Dupin", "subtitle": "A launcher", "keywords": ["launcher"], "url": "https://github.com/x-ryoppippi/dupin", "section": "Projects" },
  { "id": "rune", "title": "Rune", "subtitle": "A UI toolkit", "url": "https://github.com/anomalyco/rune", "section": "Projects" },
  { "id": "book", "title": "The Rust Book", "subtitle": "Reading", "url": "https://doc.rust-lang.org/book/" },
  { "id": "nomicon", "title": "The Rustonomicon", "subtitle": "Reading", "url": "https://doc.rust-lang.org/nomicon/" }
]"#;
    let (_folder, extension) = package(
        "",
        &[
            ("rows.js", &format!("export default {rows};\n")),
            (
                "main.js",
                "import rows from './rows.js';\nexport function items() { return rows; }",
            ),
        ],
    );
    let declared: Vec<serde_json::Value> = serde_json::from_str(rows).unwrap();
    let entries = declared
        .iter()
        .map(|row| {
            let url = row["url"].as_str().unwrap().to_owned();
            Entry {
                item: Item {
                    id: row["id"].as_str().unwrap().into(),
                    view: View::Text {
                        title: row["title"].as_str().unwrap().into(),
                        subtitle: row["subtitle"].as_str().unwrap().into(),
                    },
                    payload: Payload::default(),
                    record_usage: true,
                    icon: None,
                    matched: Vec::new(),
                    completion: None,
                    score: None,
                    primary: Some("open".into()),
                    alternate: None,
                    actions: vec![ItemAction::new("open", "Open URL", Action::OpenUrl(url))],
                },
                keywords: row["keywords"]
                    .as_array()
                    .map(|keywords| keywords.iter().map(|k| k.as_str().unwrap().to_owned()).collect())
                    .unwrap_or_default(),
                section: row["section"].as_str().map(Into::into),
                detail: None,
                web_search: None,
                match_text: None,
                time: None,
                file: None,
                file_actions: Vec::new(),
            }
        })
        .collect();
    let mut native = Catalogue::default();
    native.apply(Change::Reset(entries), catalogue::LIMITS).unwrap();
    let native = NativeCatalogue(native);
    let port = ExtensionProvider::new(extension);
    Parity {
        native: &native,
        extension: &port,
        queries: &["", "d", "launch", "rust", "rn", "zzz"],
        limit: 50,
    }
    .assert();
    assert_eq!(seen(&port, "rust", 50).len(), 2);
}

#[test]
fn the_latency_fixture_lists_five_thousand_rows() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/latency/catalogue.extensions/catalogue");
    let extension = super::load_folder(&root).unwrap();
    assert!(extension.default_search);
    let provider = ExtensionProvider::new(extension.clone());
    assert_eq!(
        batches(&provider, "")
            .iter()
            .map(|batch| batch.items.len())
            .sum::<usize>(),
        50
    );
    assert_eq!(extension.catalogue.len(), 5_000);
    assert!(!titles_of(&provider, "term").is_empty());
}

/// The bundled clipboard history in `folder`, granted what it declares as a bundled package is,
/// over scratch storage that starts with `seeded` records, oldest first.
struct History {
    _storage: tempfile::TempDir,
    extension: Arc<Extension>,
    bridge: dupin_extension_bridge::Bridge,
}

impl History {
    fn new(folder: &Path, seeded: Vec<serde_json::Value>) -> Self {
        let root = tempfile::tempdir().unwrap();
        let extension = super::load_folder(folder).unwrap();
        let storage = dupin_storage::Storage::new(root.path(), dupin_storage::MemoryKey::random());
        let namespace = dupin_extension_bridge::storage::namespace(
            &extension.package,
            dupin_extension_bridge::Origin::Bundled,
        );
        let store = storage
            .open(&namespace, dupin_storage::Limits::default())
            .unwrap();
        for data in seeded {
            store
                .put(dupin_storage::NewRecord {
                    data,
                    blobs: Vec::new(),
                })
                .unwrap();
        }
        drop(store);
        let bridge = dupin_extension_bridge::Bridge::new(
            dupin_extension_bridge::Broker::new(Default::default(), Default::default()),
            None,
            dupin_extension_bridge::Services {
                storage: Box::new(dupin_extension_bridge::PackageStorage::new(storage)),
                ..dupin_extension_bridge::Services::none()
            },
        );
        bridge.set_bundled(&extension.package, extension.revision.clone(), true);
        bridge.install_bundled(
            &extension.package,
            extension.revision.clone(),
            extension.capabilities.clone(),
        );
        Self {
            _storage: root,
            extension,
            bridge,
        }
    }

    /// The titles of the newest rows matching `query`, with time enough for a debug build to
    /// read and decrypt a large history.
    fn titles(&self, query: &str) -> Vec<String> {
        let call = self
            .bridge
            .call(&self.extension.package, dupin_extension_bridge::Trigger::Query);
        let batches = self
            .extension
            .catalogue(
                query,
                50,
                Instant::now() + Duration::from_secs(60),
                &call,
                Role::Keyword,
            )
            .unwrap();
        titles(&batches).into_iter().map(str::to_owned).collect()
    }

    fn copy(&self, text: &str) {
        let call = self.bridge.call(
            &self.extension.package,
            dupin_extension_bridge::Trigger::Background,
        );
        self.extension
            .event(
                "clipboard",
                &serde_json::json!({ "type": "text", "text": text }),
                Instant::now() + Duration::from_secs(60),
                &call.services,
            )
            .unwrap();
    }
}

fn clipboard_history() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions/clipboard-history")
}

fn now_ms() -> u64 {
    dupin_core::session::unix_now() * 1000
}

#[test]
fn the_clipboard_history_lists_copies_past_what_one_answer_or_a_catalogue_carries() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let now = now_ms();
    let mut seeded: Vec<serde_json::Value> = (0..140)
        .map(|index| {
            let body = "the log said nothing useful at all ".repeat(1700);
            serde_json::json!({
                "kind": "text",
                "text": format!("copy{index} {body} tail{index}"),
                "copiedAt": now - 1000,
            })
        })
        .collect();
    seeded.push(serde_json::json!({
        "kind": "text",
        "text": "吾輩は猫である名前はまだ無い".repeat(30),
        "copiedAt": now - 500,
    }));
    let files: Vec<_> = (0..40)
        .map(|index| {
            let name = format!("Quarterly report {index} final version.pdf");
            serde_json::json!({ "path": format!("/Users/Shared/{name}"), "name": name, "folder": "/Users/Shared" })
        })
        .collect();
    seeded.push(serde_json::json!({ "kind": "files", "files": files, "copiedAt": now }));
    let history = History::new(&clipboard_history(), seeded);

    assert_eq!(
        history.titles("tail139").len(),
        1,
        "the newest copies are found by all their text"
    );
    assert_eq!(history.extension.catalogue.len(), 142, "every copy is listed");
    let japanese = history.titles("吾輩");
    assert_eq!(japanese.len(), 1);
    assert!(
        japanese[0].len() <= 512 && japanese[0].ends_with('…'),
        "{}",
        japanese[0]
    );
    let report = history.titles("quarterly");
    assert_eq!(report.len(), 1);
    assert!(
        report[0].len() <= 512 && report[0].ends_with('…'),
        "{}",
        report[0]
    );
}

#[test]
fn copying_back_and_forth_keeps_both_copies_in_the_clipboard_history() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let history = History::new(&clipboard_history(), Vec::new());
    history.copy("foo");
    assert_eq!(history.titles(""), ["foo"]);
    history.copy("bar");
    history.copy("foo");
    assert_eq!(history.titles(""), ["foo", "bar"]);
    history.copy("bar");
    assert_eq!(history.titles(""), ["bar", "foo"]);
}

#[test]
fn a_full_clipboard_history_gives_up_its_oldest_copies_for_new_text() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let folder = tempfile::tempdir().unwrap();
    let source = clipboard_history();
    let mut manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(source.join("package.json")).unwrap()).unwrap();
    manifest["dupin"]["capabilities"]["storage"]["quotaBytes"] = serde_json::json!(16 * 1024);
    std::fs::write(folder.path().join("package.json"), manifest.to_string()).unwrap();
    std::fs::copy(source.join("main.ts"), folder.path().join("main.ts")).unwrap();
    let history = History::new(folder.path(), Vec::new());
    for index in 0..20 {
        history.copy(&format!("copy{index} {}", "lorem ipsum ".repeat(250)));
    }
    let listed = history.titles("copy");
    assert!(listed[0].starts_with("copy19 "), "{listed:?}");
    assert!(listed.len() < 20, "{listed:?}");
    assert!(history.titles("copy0").is_empty());
}

#[test]
fn search_answers_before_anything_is_typed_only_when_the_package_asks() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let source = "export function search(query) { return [{ title: `Upcoming ${JSON.stringify(query)}` }]; }";
    let (_folder, waits) = package("", &[("main.js", source)]);
    assert!(!waits.empty_query);
    assert!(titles_of(&ExtensionProvider::new(waits), " ").is_empty());

    let (_folder, eager) = package(r#", "emptyQuery": true"#, &[("main.js", source)]);
    assert!(eager.empty_query);
    let provider = ExtensionProvider::new(eager);
    assert_eq!(titles_of(&provider, ""), [r#"Upcoming """#]);
    assert_eq!(titles_of(&provider, "rev"), [r#"Upcoming "rev""#]);
}
