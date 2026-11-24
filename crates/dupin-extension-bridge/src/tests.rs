//! End to end through the sandboxed helper: guest JavaScript calls the `dupin` module, the
//! request crosses to this process, the broker decides and a fixture service answers.

use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use dupin_extension_host::{ActionEffect, Compiled, Host};
use dupin_js_runtime::{DEFAULT_ENTRY, RunError, Sources};
use dupin_plugin_protocol::{PluginItem, View};

use super::services::fixture;
use super::*;

const HELPER_ENV: &str = "DUPIN_EXTENSION_BRIDGE_TEST";

/// Not a test: the helper these tests start is this test binary running only this function.
#[test]
fn helper_entry() {
    if std::env::var_os(HELPER_ENV).is_some() {
        dupin_extension_host::serve();
    }
}

fn host() -> Host {
    Host::new(Box::new(|| {
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args([
                "--exact",
                "tests::helper_entry",
                "--nocapture",
                "--test-threads=1",
            ])
            .env_clear()
            .env(HELPER_ENV, "1")
            .current_dir("/");
        Ok(command)
    }))
}

struct World {
    host: Host,
    bridge: Bridge,
    id: PackageId,
    package: Compiled,
    http: Arc<fixture::Http>,
    exec: Arc<fixture::Exec>,
    storage: Arc<fixture::Storage>,
    clipboard: Arc<fixture::Clipboard>,
    folder: tempfile::TempDir,
}

fn revision() -> Revision {
    Revision::new("sha256:fixture").unwrap()
}

fn world(capabilities: serde_json::Value, source: &str) -> World {
    world_with(capabilities, source, Base::default(), |_| {})
}

fn world_with(
    capabilities: serde_json::Value,
    source: &str,
    base: Base,
    adjust: impl FnOnce(&mut World),
) -> World {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join(DEFAULT_ENTRY), source).unwrap();
    let sources = Sources::read(folder.path(), DEFAULT_ENTRY).unwrap();
    let host = host();
    let package = host.compile(&sources, Duration::from_secs(5), &|| false).unwrap();
    let http = Arc::new(fixture::Http {
        body: r#"{"archived":false}"#.into(),
        ..fixture::Http::default()
    });
    let exec = Arc::new(fixture::Exec {
        stdout: "clean\n".into(),
        ..fixture::Exec::default()
    });
    let storage = Arc::new(fixture::Storage::default());
    let clipboard = Arc::new(fixture::Clipboard::default());
    let services = Services {
        http: Box::new(http.clone()),
        exec: Box::new(exec.clone()),
        storage: Box::new(storage.clone()),
        clipboard: Box::new(clipboard.clone()),
        dictionary: Box::new(fixture::Dictionary),
        calendar: Box::new(fixture::Calendar { today: 0 }),
    };
    let grants = folder.path().join("grants.toml");
    let bridge = Bridge::new(Broker::new(base, Local::default()), Some(grants), services);
    let id = PackageId::new("weather").unwrap();
    let capabilities = Capabilities::parse(&capabilities).unwrap();
    bridge.install(&id, revision(), capabilities);
    let mut world = World {
        host,
        bridge,
        id,
        package,
        http,
        exec,
        storage,
        clipboard,
        folder,
    };
    adjust(&mut world);
    world
}

impl World {
    fn search(&self, query: &str) -> Result<Vec<PluginItem>, RunError> {
        let call = self.bridge.call(&self.id, Trigger::Query);
        self.host.search(
            &self.package,
            query,
            10,
            Instant::now() + Duration::from_secs(5),
            &|| false,
            &call.services,
        )
    }

    fn act(&self, name: &str, data: serde_json::Value) -> Result<Option<ActionEffect>, RunError> {
        let call = self.bridge.call(&self.id, Trigger::Activation);
        self.host.action(
            &self.package,
            name,
            &data,
            Instant::now() + Duration::from_secs(5),
            &|| false,
            &call.services,
        )
    }

    fn first(&self, query: &str) -> String {
        let rows = self.search(query).unwrap();
        match &rows[0].view {
            View::Text { title, .. } => title.clone(),
            other => panic!("{other:?}"),
        }
    }

    fn toast(&self, name: &str) -> String {
        match self.act(name, serde_json::Value::Null).unwrap() {
            Some(ActionEffect::Toast { text }) => text,
            other => panic!("{other:?}"),
        }
    }

    fn enable(&self) {
        self.bridge.set_enabled(&self.id, true).unwrap();
    }

    fn grant(&self, key: &str) {
        let key: ScopeKey = key.parse().unwrap();
        let scope = self
            .bridge
            .scopes(&self.id)
            .into_iter()
            .find(|entry| entry.scope.key() == key)
            .unwrap()
            .scope;
        let binding = scope.needs_binding().then(|| self.folder.path().to_owned());
        self.bridge.allow(&self.id, &revision(), &scope, binding).unwrap();
    }
}

/// Each guest reports the outcome of its request as `ok …` or `code: message`.
const REPORT: &str = r#"
const report = async (run) => { try { return `ok ${JSON.stringify(await run())}`; } catch (error) { return `${error.code}`; } };
"#;

fn guest(body: &str) -> String {
    format!(
        "import {{ http, files, exec, storage, clipboard, dictionary, calendar }} from 'dupin';\n{REPORT}\n{body}"
    )
}

#[test]
fn an_allowed_request_reaches_its_service_and_returns_the_result() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "http": [{ "origin": "https://api.github.com", "methods": ["GET"] }] }),
        &guest(
            "export async function search() {
               const response = await http.fetch('https://api.github.com/repos/x/y', { headers: { accept: 'application/json' } });
               const repo = await response.json();
               return [{ title: `${response.status} ${repo.archived ? 'Archived' : 'Active repository'}` }];
             }",
        ),
    );
    world.grant("http:https://api.github.com");
    assert_eq!(world.first(""), "200 Active repository");
    let requests = world.http.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "https://api.github.com/repos/x/y");
    assert_eq!(
        requests[0].headers,
        [("accept".to_owned(), "application/json".to_owned())]
    );
    // The grant was saved for the next launch.
    let saved = std::fs::read_to_string(world.folder.path().join("grants.toml")).unwrap();
    assert!(saved.contains("decision = \"granted\""), "{saved}");
}

#[test]
fn undeclared_and_base_denied_requests_are_refused() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let base =
        Base::parse("version = 1\n[packages.weather]\nenabled = true\ndeny = [\"dictionary\"]\n").unwrap();
    let world = world_with(
        serde_json::json!({ "dictionary": ["lookup"] }),
        &guest(
            "export async function search() {
               return [{ title: [await report(() => dictionary.lookup('word')), await report(() => http.fetch('https://example.com'))].join(' ') }];
             }",
        ),
        base,
        |_| {},
    );
    assert_eq!(world.first(""), "denied undeclared");
    assert!(world.bridge.pending(None).is_empty());
    assert!(world.http.requests.lock().unwrap().is_empty());
}

#[test]
fn a_disabled_package_gets_nothing() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "dictionary": ["lookup"] }),
        &guest(
            "export async function search() { return [{ title: await report(() => dictionary.lookup('word')) }]; }",
        ),
    );
    assert_eq!(world.first(""), "disabled");
}

#[test]
fn a_request_without_consent_fails_and_waits_for_the_user_who_runs_nothing_by_granting() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "storage": { "quotaBytes": 1024 } }),
        &guest(
            "export async function search(query) { return [{ title: await report(() => storage.set('last', query)) }]; }",
        ),
    );
    world.enable();
    assert_eq!(world.first("hello"), "needs-consent");
    let pending = world.bridge.pending(Some(&world.id));
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].key(), ScopeKey::Storage);
    assert_eq!(pending[0].reason, ConsentReason::NotGranted);
    assert_eq!(pending[0].description, "keep up to 1024 bytes of encrypted data");
    assert!(world.storage.data.lock().unwrap().is_empty());

    world
        .bridge
        .allow(&world.id, &pending[0].revision, &pending[0].scope, None)
        .unwrap();
    assert!(world.bridge.pending(None).is_empty());
    assert!(
        world.storage.data.lock().unwrap().is_empty(),
        "granting re-runs nothing"
    );
    assert_eq!(world.first("hello"), "ok undefined");
    assert_eq!(
        world.storage.data.lock().unwrap()["weather"]["last"],
        serde_json::json!("hello")
    );

    // A consent for another revision than the installed one is refused.
    assert_eq!(
        world.bridge.allow(
            &world.id,
            &Revision::new("sha256:other").unwrap(),
            &pending[0].scope,
            None
        ),
        Err(BrokerError::StaleRevision)
    );
}

#[test]
fn the_guest_reads_what_consent_needs_from_its_error() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "clipboard": ["read"] }),
        &guest(
            "export const search = () => [];
             export const actions = { async paste() {
               try { await clipboard.readText(); return { type: 'toast', text: 'read' }; }
               catch (error) { return { type: 'toast', text: `${error.code} ${error.scope} ${error.permission} ${error.description}` }; }
             } };",
        ),
    );
    world.enable();
    world.grant("clipboard:read");
    // Granted, but macOS hasn't decided whether Dupin may read the pasteboard.
    assert_eq!(
        world.toast("paste"),
        "needs-consent clipboard:read Paste from Other Apps read the clipboard"
    );
    assert_eq!(
        world.bridge.pending(None)[0].reason,
        ConsentReason::OsAccess(OsPermission::Pasteboard)
    );
    world
        .bridge
        .set_os_access(OsPermission::Pasteboard, OsAccess::Allowed);
    *world.clipboard.text.lock().unwrap() = Some("copied".into());
    assert_eq!(world.toast("paste"), "read");
    world
        .bridge
        .set_os_access(OsPermission::Pasteboard, OsAccess::Denied);
    assert!(world.toast("paste").starts_with("os-denied"));
}

#[test]
fn mutations_and_programs_need_an_action_the_user_chose() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({
            "clipboard": ["write"],
            "http": [{ "origin": "https://api.example.com", "methods": ["GET", "POST"] }],
            "exec": { "git-status": { "args": ["status", "--short", "{path}"], "output": true } }
        }),
        &guest(
            "const attempts = () => Promise.all([
               report(() => clipboard.writeText('x')),
               report(() => http.fetch('https://api.example.com/items', { method: 'POST', body: '{}' }).then((r) => r.status)),
               report(() => exec.run('git-status', { path: 'src' }).then((r) => `${r.code} ${r.stdout.trim()}`)),
             ]).then((results) => results.join(' | '));
             export async function search() { return [{ title: await attempts() }]; }
             export const actions = { async go() { return { type: 'toast', text: await attempts() }; } };",
        ),
    );
    world.enable();
    for key in [
        "clipboard:write",
        "http:https://api.example.com",
        "exec:git-status",
    ] {
        world.grant(key);
    }
    assert_eq!(
        world.first(""),
        "needs-activation | needs-activation | needs-activation"
    );
    assert!(world.exec.runs.lock().unwrap().is_empty());
    assert_eq!(world.toast("go"), "ok undefined | ok 200 | ok \"0 clean\"");
    assert_eq!(*world.clipboard.text.lock().unwrap(), Some("x".to_owned()));
    let runs = world.exec.runs.lock().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].0, "git-status");
}

#[test]
fn a_write_past_the_quota_is_refused() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "storage": { "quotaBytes": 64 } }),
        &guest(
            "export async function search() {
               const small = await report(() => storage.set('a', 'x'.repeat(20)));
               const large = await report(() => storage.set('b', 'x'.repeat(60)));
               const replaced = await report(() => storage.set('a', 'y'.repeat(30)));
               return [{ title: `${small} ${large} ${replaced} ${JSON.stringify(await storage.keys())}` }];
             }",
        ),
    );
    world.enable();
    world.grant("storage");
    assert_eq!(
        world.first(""),
        "ok undefined quota-exceeded ok undefined [\"a\"]"
    );
}

#[test]
fn revoking_while_a_request_runs_drops_its_result_and_the_call_is_stale() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "http": [{ "origin": "https://api.example.com", "methods": ["GET"] }] }),
        &guest(
            "export async function search() {
               return [{ title: await report(() => http.fetch('https://api.example.com/slow').then((r) => r.status)) }];
             }",
        ),
    );
    world.enable();
    world.grant("http:https://api.example.com");
    *world.http.delay.lock().unwrap() = Duration::from_millis(300);
    let world = Arc::new(world);
    let call = world.bridge.call(&world.id, Trigger::Query);
    let searching = {
        let world = world.clone();
        let services = call.services.clone();
        std::thread::spawn(move || {
            world.host.search(
                &world.package,
                "",
                10,
                Instant::now() + Duration::from_secs(5),
                &|| false,
                &services,
            )
        })
    };
    std::thread::sleep(Duration::from_millis(100));
    world
        .bridge
        .revoke(&world.id, &"http:https://api.example.com".parse().unwrap())
        .unwrap();
    let rows = searching.join().unwrap().unwrap();
    let View::Text { title, .. } = &rows[0].view else {
        unreachable!()
    };
    assert_eq!(title, "revoked");
    assert_eq!(
        world.http.finished.load(Ordering::Relaxed),
        1,
        "the fetch finished"
    );
    assert!(!world.bridge.is_current(&world.id, call.generation));

    // Granting again doesn't revive the old call.
    world.grant("http:https://api.example.com");
    assert!(!world.bridge.is_current(&world.id, call.generation));
}

#[test]
fn hostile_requests_are_refused_before_any_service_runs() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({
            "http": [{ "origin": "https://api.example.com", "methods": ["GET"] }],
            "files": { "notes": { "kind": "directory", "access": ["read", "list"] } },
            "storage": { "quotaBytes": 1024 },
            "calendar": { "access": ["read"], "pastDays": 1, "futureDays": 7 }
        }),
        &guest(
            "export async function search() {
               const results = await Promise.all([
                 report(() => http.fetch('http://api.example.com/')),
                 report(() => http.fetch('https://api.example.com.evil.test/')),
                 report(() => http.fetch('https://api.example.com/', { method: 'TRACE' })),
                 report(() => files.readText('notes', '../../etc/passwd')),
                 report(() => files.readText('notes', '/etc/passwd')),
                 report(() => files.readText('Notes!', 'a')),
                 report(() => storage.get('')),
                 report(() => storage.set('k', undefined)),
                 report(() => calendar.events({ from: 0, to: 30 * 864e5 })),
                 report(() => calendar.events({ from: NaN, to: 1 })),
               ]);
               return [{ title: results.join(' ') }];
             }",
        ),
    );
    world.enable();
    for key in [
        "http:https://api.example.com",
        "files:notes",
        "storage",
        "calendar",
    ] {
        world.grant(key);
    }
    world
        .bridge
        .set_os_access(OsPermission::Calendars, OsAccess::Allowed);
    assert_eq!(
        world.first(""),
        "invalid undeclared invalid invalid invalid invalid invalid invalid out-of-scope invalid"
    );
    assert!(world.http.requests.lock().unwrap().is_empty());

    // Straight at the session, as a subverted helper might send them.
    let call = world.bridge.call(&world.id, Trigger::Query);
    let ask = |op: &str, args: &[u8]| -> serde_json::Value {
        serde_json::from_slice(&call.services.call(
            op,
            args,
            Instant::now() + Duration::from_secs(1),
            &Default::default(),
        ))
        .unwrap()
    };
    for (op, args) in [
        ("storage.get", &b"not json"[..]),
        ("storage.get", br#"{"key":"k","extra":1}"#),
        ("storage.get", br#"{"key":7}"#),
        ("files.read", br#"{"scope":"notes","path":"a\u0000b"}"#),
        ("exec.run", br#"{"profile":"../bin/sh"}"#),
        ("dictionary.lookup", br#"{"word":"\u0007"}"#),
        ("calendar.events", br#"{"from":1,"to":0}"#),
    ] {
        assert_eq!(
            ask(op, args)["error"]["code"],
            "invalid",
            "{op} {}",
            String::from_utf8_lossy(args)
        );
    }
    assert_eq!(ask("process.spawn", b"{}")["error"]["code"], "unknown-operation");
}

#[test]
fn files_read_and_list_only_inside_the_folder_the_user_picked() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "files": { "notes": { "kind": "directory", "access": ["read", "list"] } } }),
        &guest(
            "export async function search() {
               const names = (await files.list('notes')).map((entry) => `${entry.name}:${entry.kind}`).sort();
               return [{ title: `${names.join(',')} ${await files.readText('notes', 'today.md')}` }];
             }",
        ),
    );
    std::fs::write(world.folder.path().join("today.md"), "Buy milk").unwrap();
    world.enable();
    world.grant("files:notes");
    let title = world.first("");
    assert!(title.contains("today.md:file"), "{title}");
    assert!(title.ends_with(" Buy milk"), "{title}");
}

#[test]
fn calendar_reads_stay_inside_the_declared_window() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "calendar": { "access": ["read"], "pastDays": 0, "futureDays": 2 } }),
        &guest(
            "export async function search() {
               const day = 864e5;
               const inside = await report(() => calendar.events({ from: 0, to: 2 * day }).then((events) => events.map((e) => `${e.title}@${e.start}`)));
               const past = await report(() => calendar.events({ from: -day, to: day }));
               return [{ title: `${inside} ${past}` }];
             }",
        ),
    );
    world.enable();
    world.grant("calendar");
    world
        .bridge
        .set_os_access(OsPermission::Calendars, OsAccess::Allowed);
    assert_eq!(
        world.first(""),
        "ok [\"Standup@1970-01-01T12:00:00.000Z\",\"Standup@1970-01-02T12:00:00.000Z\"] out-of-scope"
    );
}

#[test]
fn dictionary_lookups_return_entries() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "dictionary": ["lookup"] }),
        &guest(
            "export async function search(query) { const [entry] = await dictionary.lookup(query); return [{ title: entry.groups[0].senses[0].definition }]; }",
        ),
    );
    world.enable();
    world.grant("dictionary");
    assert_eq!(world.first("dupin"), "the word dupin");
}

#[test]
fn clipboard_changes_reach_the_handler_while_the_package_may_watch() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = Arc::new(world(
        serde_json::json!({ "clipboard": ["watch"], "storage": { "quotaBytes": 4096 } }),
        &guest(
            "export const search = () => [];
             export const events = { async clipboard(change) {
               const seen = (await storage.get('seen')) ?? [];
               await storage.set('seen', [...seen, change.text]);
             } };",
        ),
    ));
    let delivered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    {
        let (world_for_watch, delivered) = (Arc::downgrade(&world), delivered.clone());
        world.bridge.watch_clipboard(&world.id, move |event| {
            let Some(world) = world_for_watch.upgrade() else {
                return;
            };
            let call = world.bridge.call(&world.id, Trigger::Background);
            let _ = world.host.event(
                &world.package,
                "clipboard",
                &event,
                Instant::now() + Duration::from_secs(5),
                &|| false,
                &call.services,
            );
            delivered.fetch_add(1, Ordering::Relaxed);
        });
    }
    assert_eq!(world.clipboard.watchers(), 0, "not on, not granted");
    world.enable();
    world.grant("clipboard:watch");
    world.grant("storage");
    assert_eq!(world.clipboard.watchers(), 0, "macOS hasn't allowed it");
    world
        .bridge
        .set_os_access(OsPermission::Pasteboard, OsAccess::Allowed);
    assert_eq!(world.clipboard.watchers(), 1);

    world.clipboard.copy("first");
    wait_for(|| delivered.load(Ordering::Relaxed) == 1);
    for index in 0..20 {
        world.clipboard.copy(&format!("burst {index}"));
    }
    wait_for(|| {
        world.storage.data.lock().unwrap()["weather"]["seen"]
            .as_array()
            .is_some_and(|seen| seen.last() == Some(&serde_json::json!("burst 19")))
    });
    let seen = world.storage.data.lock().unwrap()["weather"]["seen"].clone();
    let seen = seen.as_array().unwrap();
    assert_eq!(seen[0], "first");
    assert!(seen.len() < 21, "a burst is coalesced: {seen:?}");

    world
        .bridge
        .revoke(&world.id, &"clipboard:watch".parse().unwrap())
        .unwrap();
    assert_eq!(world.clipboard.watchers(), 0);
    let before = delivered.load(Ordering::Relaxed);
    world.clipboard.copy("after revoking");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(delivered.load(Ordering::Relaxed), before);
}

fn wait_for(done: impl Fn() -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < until, "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn an_unreadable_grant_file_grants_nothing_and_is_never_overwritten() {
    let folder = tempfile::tempdir().unwrap();
    let local = folder.path().join("grants.toml");
    std::fs::write(&local, "not = [valid").unwrap();
    let bridge = Bridge::open(&folder.path().join("base.toml"), &local, Services::none());
    assert_eq!(bridge.problems().len(), 1);
    let id = PackageId::new("weather").unwrap();
    bridge.install(
        &id,
        revision(),
        Capabilities::parse(&serde_json::json!({ "dictionary": ["lookup"] })).unwrap(),
    );
    assert_eq!(bridge.set_enabled(&id, true), Err(BrokerError::GrantsUnreadable));
    assert_eq!(std::fs::read_to_string(&local).unwrap(), "not = [valid");
}

/// A bridge over grant files in `folder`, with `weather` declaring the dictionary and
/// `notes` declaring nothing, both on and `weather`'s dictionary allowed.
fn granted_on_disk(folder: &Path) -> (Bridge, PackageId, PackageId, Arc<Mutex<Vec<Notice>>>) {
    let bridge = Bridge::open(
        &folder.join("base.toml"),
        &folder.join("grants.toml"),
        Services::none(),
    );
    let notices = Arc::new(Mutex::new(Vec::new()));
    bridge.set_listener({
        let notices = notices.clone();
        move |notice| notices.lock().unwrap().push(notice)
    });
    let (weather, notes) = (
        PackageId::new("weather").unwrap(),
        PackageId::new("notes").unwrap(),
    );
    let dictionary = Capabilities::parse(&serde_json::json!({ "dictionary": ["lookup"] })).unwrap();
    bridge.install(&weather, revision(), dictionary);
    bridge.install(&notes, revision(), Capabilities::default());
    let scope = bridge.scopes(&weather).remove(0).scope;
    bridge.allow(&weather, &revision(), &scope, None).unwrap();
    bridge.set_enabled(&notes, true).unwrap();
    assert_eq!(
        bridge.scopes(&weather)[0].state,
        ConsentState::Granted(GrantSource::Local)
    );
    notices.lock().unwrap().clear();
    (bridge, weather, notes, notices)
}

#[test]
fn grants_another_process_forgets_stay_forgotten() {
    let folder = tempfile::tempdir().unwrap();
    let (bridge, weather, notes, notices) = granted_on_disk(folder.path());

    // What `dupin ext remove weather` does while Dupin runs.
    let local = folder.path().join("grants.toml");
    let mut other = Broker::new(
        Base::default(),
        Local::parse(&std::fs::read_to_string(&local).unwrap()).unwrap(),
    );
    other.uninstall(&weather);
    other.save(&local).unwrap();

    // Any save of the app's own must not write the removed grants back.
    bridge.set_enabled(&notes, false).unwrap();
    let saved = std::fs::read_to_string(&local).unwrap();
    assert!(!saved.contains("weather"), "{saved}");
    assert!(saved.contains("notes"), "{saved}");
    assert!(!bridge.is_enabled(&weather));
    assert_eq!(bridge.scopes(&weather)[0].state, ConsentState::Undecided);
    assert!(
        notices
            .lock()
            .unwrap()
            .contains(&Notice::Changed(weather.clone()))
    );

    // A package that later takes the name inherits nothing.
    let other_weather = Capabilities::parse(&serde_json::json!({ "dictionary": ["lookup"] })).unwrap();
    bridge.install(&weather, Revision::new("sha256:other").unwrap(), other_weather);
    assert!(!bridge.is_enabled(&weather));
    assert_eq!(bridge.scopes(&weather)[0].state, ConsentState::Undecided);
}

#[test]
fn a_declared_base_changed_while_running_applies_its_denials_at_once() {
    let folder = tempfile::tempdir().unwrap();
    let (bridge, weather, notes, notices) = granted_on_disk(folder.path());
    let call = bridge.call(&weather, Trigger::Activation);

    std::fs::write(
        folder.path().join("base.toml"),
        "version = 1\n[packages.weather]\ndeny = [\"dictionary\"]\n[packages.notes]\nenabled = false\n",
    )
    .unwrap();
    bridge.reload();
    assert_eq!(bridge.scopes(&weather)[0].state, ConsentState::DeniedByBase);
    assert!(!bridge.is_enabled(&notes), "a declared `enable = false` holds");
    assert!(
        !bridge.is_current(&weather, call.generation),
        "in-flight results are dropped"
    );
    let changed = notices.lock().unwrap().clone();
    assert!(changed.contains(&Notice::Changed(weather.clone())), "{changed:?}");
    assert!(changed.contains(&Notice::Changed(notes.clone())), "{changed:?}");

    notices.lock().unwrap().clear();
    bridge.reload();
    assert!(
        notices.lock().unwrap().is_empty(),
        "an unchanged file is not read again"
    );
}
