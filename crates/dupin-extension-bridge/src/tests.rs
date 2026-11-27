//! End to end through the sandboxed helper: guest JavaScript calls the `dupin` module, the
//! request crosses to this process, the broker decides and a fixture service answers. Storage
//! is the real encrypted store in a temporary folder, under an in-memory key.

use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dupin_extension_host::{ActionEffect, Compiled, Host, Trust};
use dupin_js_runtime::{Acted, DEFAULT_ENTRY, Row, RunError, Sources};

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

#[derive(Clone)]
struct FakeClock(Arc<AtomicU64>);

impl dupin_storage::Clock for FakeClock {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}

struct World {
    host: Host,
    bridge: Bridge,
    id: PackageId,
    package: Compiled,
    http: Arc<fixture::Http>,
    exec: Arc<fixture::Exec>,
    storage: Arc<PackageStorage>,
    clock: FakeClock,
    clipboard: Arc<fixture::Clipboard>,
    windows: Arc<fixture::Windows>,
    snippets: fixture::Snippets,
    grants: MemoryLocal,
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
    let package = host
        .compile(&sources, Trust::Own, Duration::from_secs(5), &|| false)
        .unwrap();
    let http = Arc::new(fixture::Http {
        body: r#"{"archived":false}"#.into(),
        ..fixture::Http::default()
    });
    let exec = Arc::new(fixture::Exec {
        stdout: "clean\n".into(),
        ..fixture::Exec::default()
    });
    let clock = FakeClock(Arc::new(AtomicU64::new(1_700_000_000_000)));
    let storage = Arc::new(PackageStorage::new(
        dupin_storage::Storage::new(folder.path().join("storage"), dupin_storage::MemoryKey::random())
            .with_clock(clock.clone()),
    ));
    let clipboard = Arc::new(fixture::Clipboard::default());
    let windows = Arc::new(fixture::Windows::default());
    let snippets = fixture::Snippets::default();
    let services = Services {
        http: Box::new(http.clone()),
        exec: Box::new(exec.clone()),
        storage: Box::new(storage.clone()),
        clipboard: Box::new(clipboard.clone()),
        dictionary: Box::new(fixture::Dictionary),
        calculator: Box::new(fixture::Calculator),
        snippets: Box::new(snippets.clone()),
        calendar: Box::new(fixture::Calendar { today: 0 }),
        windows: Box::new(windows.clone()),
        context: Box::new(fixture::Context),
    };
    let grants = MemoryLocal::default();
    let bridge = Bridge::new(
        Broker::new(base, Local::default()),
        Some(Box::new(grants.clone())),
        services,
    );
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
        clock,
        clipboard,
        windows,
        snippets,
        grants,
        folder,
    };
    adjust(&mut world);
    world
}

impl World {
    fn search(&self, query: &str) -> Result<Vec<Row>, RunError> {
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

    fn act(&self, name: &str, data: serde_json::Value) -> Result<Acted, RunError> {
        let call = self.bridge.call(&self.id, Trigger::Activation);
        self.host.action(
            &self.package,
            name,
            &data,
            &dupin_extension_host::PageContext::default(),
            &serde_json::json!({}),
            Instant::now() + Duration::from_secs(5),
            &|| false,
            &call.services,
        )
    }

    fn first(&self, query: &str) -> String {
        self.search(query).unwrap().remove(0).title
    }

    fn toast(&self, name: &str) -> String {
        match self.act(name, serde_json::Value::Null).unwrap().effect {
            Some(ActionEffect::Toast { text }) => text,
            other => panic!("{other:?}"),
        }
    }

    fn enable(&self) {
        self.bridge.set_enabled(&self.id, true).unwrap();
    }

    /// The package's namespace on disk, as a user's copy.
    fn namespace_dir(&self) -> std::path::PathBuf {
        self.folder
            .path()
            .join("storage")
            .join(storage::namespace(&self.id, Origin::User))
    }

    /// The data of each record the package stored as a user's copy, newest first, read with the
    /// limits it declares.
    fn stored(&self) -> Vec<serde_json::Value> {
        let Some(Scope::Storage {
            quota_bytes,
            retention_days,
        }) = self
            .bridge
            .scopes(&self.id)
            .into_iter()
            .map(|entry| entry.scope)
            .find(|scope| matches!(scope, Scope::Storage { .. }))
        else {
            panic!("the package declares no storage");
        };
        let store = services::Storage::open(
            &self.storage,
            &storage::namespace(&self.id, Origin::User),
            storage::limits(quota_bytes, retention_days),
            None,
        )
        .unwrap();
        let records = store.with(|store| store.list(None, 200)).unwrap().records;
        records.into_iter().map(|record| record.data).collect()
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

/// A request made straight at a session, whose run never ends.
const ASKED: HostRequest = HostRequest { run: 0, call: 1 };

fn guest(body: &str) -> String {
    format!(
        "import {{ http, files, exec, storage, clipboard, dictionary, calculator, snippets, calendar, windows, context }} from 'dupin';\n{REPORT}\n{body}"
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
    let saved = world.grants.records()[&world.id].to_string();
    assert!(saved.contains(r#""decision":"granted""#), "{saved}");
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
            "export async function search(query) { return [{ title: await report(() => storage.put({ last: query }).then((record) => record.data.last)) }]; }",
        ),
    );
    world.enable();
    assert_eq!(world.first("hello"), "needs-consent");
    let pending = world.bridge.pending(Some(&world.id));
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].key(), ScopeKey::Storage);
    assert_eq!(pending[0].reason, ConsentReason::NotGranted);
    assert_eq!(pending[0].description, "keep up to 1024 bytes of encrypted data");
    assert!(!world.namespace_dir().exists(), "nothing was opened");

    world
        .bridge
        .allow(&world.id, &pending[0].revision, &pending[0].scope, None)
        .unwrap();
    assert!(world.bridge.pending(None).is_empty());
    assert!(!world.namespace_dir().exists(), "granting re-runs nothing");
    assert_eq!(world.first("hello"), "ok \"hello\"");
    assert_eq!(world.stored(), [serde_json::json!({ "last": "hello" })]);

    // A consent for another revision than the installed one is refused.
    assert!(matches!(
        world.bridge.allow(
            &world.id,
            &Revision::new("sha256:other").unwrap(),
            &pending[0].scope,
            None
        ),
        Err(SaveError::Refused(BrokerError::StaleRevision))
    ));
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
fn a_write_past_the_quota_is_refused_and_deletes_nothing() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "storage": { "quotaBytes": 64 } }),
        &guest(
            "export async function search() {
               const small = await report(() => storage.put('x'.repeat(20)).then(() => 'stored'));
               const large = await report(() => storage.put('x'.repeat(60)));
               const huge = await report(() => storage.put('x'.repeat(70)));
               const blobs = await report(() => storage.put(null, { blobs: Array(17).fill(new Uint8Array(1)) }));
               const usage = await storage.usage();
               return [{ title: `${small} ${large} ${huge} ${blobs} ${usage.entries} ${usage.bytes} ${usage.quotaBytes} ${usage.retentionDays}` }];
             }",
        ),
    );
    world.enable();
    world.grant("storage");
    assert_eq!(
        world.first(""),
        "ok \"stored\" quota-exceeded quota-exceeded too-large 1 22 64 undefined"
    );
    assert_eq!(world.stored(), [serde_json::json!("x".repeat(20))]);
}

#[test]
fn concurrent_writes_never_pass_the_quota_together() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "storage": { "quotaBytes": 1000 } }),
        "export const search = () => [];",
    );
    world.enable();
    world.grant("storage");
    // Each record is 100 bytes of JSON, so ten fit.
    let args = serde_json::json!({ "data": "x".repeat(98) }).to_string();
    let barrier = Arc::new(std::sync::Barrier::new(24));
    let world = Arc::new(world);
    let writers: Vec<_> = (0..24)
        .map(|_| {
            let (world, barrier, args) = (world.clone(), barrier.clone(), args.clone());
            std::thread::spawn(move || {
                let call = world.bridge.call(&world.id, Trigger::Query);
                barrier.wait();
                let reply = call.services.call(
                    ASKED,
                    "storage.put",
                    args.as_bytes(),
                    Instant::now() + Duration::from_secs(5),
                    &Default::default(),
                );
                let reply: serde_json::Value = serde_json::from_slice(&reply).unwrap();
                match &reply["error"]["code"] {
                    serde_json::Value::Null => "stored".to_owned(),
                    code => code.as_str().unwrap().to_owned(),
                }
            })
        })
        .collect();
    let mut outcomes: Vec<String> = writers.into_iter().map(|writer| writer.join().unwrap()).collect();
    outcomes.sort();
    outcomes.dedup();
    assert_eq!(outcomes, ["quota-exceeded", "stored"]);
    assert_eq!(world.stored().len(), 10);
}

#[test]
fn records_and_blobs_round_trip_newest_first_sealed_on_disk() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "storage": { "quotaBytes": 1048576, "retentionDays": 30 } }),
        &guest(
            "export async function search() {
               const bytes = new Uint8Array(256).map((_, index) => index);
               const first = await storage.put({ n: 1, note: 'plain-marker' }, { blobs: [bytes] });
               const second = await storage.put({ n: 2 }, { blobs: [bytes.buffer] });
               await storage.put({ n: 3 });
               const one = await storage.list({ limit: 2 });
               const two = await storage.list({ before: one.next, limit: 2 });
               const blob = await storage.readBlob(first.blobs[0].handle);
               const got = await storage.get(second.id);
               const missing = await storage.get('00000000000000000000000000000001');
               const deleted = [await storage.delete(first.id), await storage.delete(first.id)];
               const kept = await storage.readBlob(second.blobs[0].handle);
               const usage = await storage.usage();
               await storage.clear();
               const gone = await report(() => storage.readBlob(first.blobs[0].handle));
               const after = await storage.list();
               return [{ title: [
                 one.records.map((record) => record.data.n), typeof one.next, two.records.map((record) => record.data.n), two.next,
                 blob.length, blob.every((byte, index) => byte === index), first.blobs[0].size,
                 first.blobs[0].handle === second.blobs[0].handle, Object.isFrozen(first), first.createdAt,
                 got.data.n, missing, deleted, kept.length,
                 usage.entries, usage.bytes, usage.quotaBytes, usage.retentionDays,
                 gone, after.records.length, after.next,
               ].join(' ') }];
             }",
        ),
    );
    world.enable();
    world.grant("storage");
    let files_with_marker = || {
        let mut found = Vec::new();
        for sub in ["records", "blobs"] {
            for entry in std::fs::read_dir(world.namespace_dir().join(sub)).unwrap() {
                let path = entry.unwrap().path();
                let bytes = std::fs::read(&path).unwrap();
                if bytes.windows(12).any(|window| window == b"plain-marker") {
                    found.push(path);
                }
            }
        }
        found
    };
    assert_eq!(
        world.first(""),
        "3,2 string 1  256 true 256 true true 2023-11-14T22:13:20.000Z 2  true,false 256 2 270 1048576 30 not-found 0 "
    );
    assert!(files_with_marker().is_empty());
}

#[test]
fn records_past_the_declared_retention_are_gone() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "storage": { "quotaBytes": 1024, "retentionDays": 1 } }),
        &guest(
            "export async function search(query) {
               if (query) await storage.put(query);
               return [{ title: (await storage.list()).records.map((record) => record.data).join(',') || 'none' }];
             }",
        ),
    );
    world.enable();
    world.grant("storage");
    assert_eq!(world.first("old"), "old");
    world.clock.0.fetch_add(12 * 60 * 60 * 1000, Ordering::SeqCst);
    assert_eq!(world.first("new"), "new,old");
    world.clock.0.fetch_add(13 * 60 * 60 * 1000, Ordering::SeqCst);
    assert_eq!(world.first(""), "new");
}

#[test]
fn revoking_keeps_the_data_and_deleting_it_removes_the_namespace() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "storage": { "quotaBytes": 1024 } }),
        &guest(
            "export async function search(query) {
               return [{ title: await report(async () => {
                 if (query) await storage.put(query);
                 return (await storage.list()).records.map((record) => record.data).join(',');
               }) }];
             }",
        ),
    );
    world.enable();
    world.grant("storage");
    assert_eq!(world.first("kept"), "ok \"kept\"");
    world.bridge.revoke(&world.id, &ScopeKey::Storage).unwrap();
    assert_eq!(world.first("refused"), "revoked");
    assert_eq!(world.stored(), [serde_json::json!("kept")]);

    world.grant("storage");
    assert_eq!(world.first(""), "ok \"kept\"");
    world.bridge.delete_data(&world.id, Origin::User).unwrap();
    assert!(!world.namespace_dir().exists());
    assert_eq!(world.first(""), "ok \"\"");
}

#[test]
fn a_users_copy_of_a_bundled_package_never_sees_its_data() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let capabilities = serde_json::json!({ "storage": { "quotaBytes": 1024 } });
    let world = world(
        capabilities.clone(),
        &guest(
            "export async function search(query) {
               if (query) await storage.put(query);
               return [{ title: (await storage.list()).records.map((record) => record.data).join(',') || 'none' }];
             }",
        ),
    );
    let bundled = Revision::new("sha256:bundled").unwrap();
    let capabilities = Capabilities::parse(&capabilities).unwrap();
    world.bridge.set_bundled(&world.id, bundled.clone(), true);
    world
        .bridge
        .install_bundled(&world.id, bundled.clone(), capabilities.clone());
    assert_eq!(world.bridge.origin(&world.id), Origin::Bundled);
    assert_eq!(world.first("shipped"), "shipped", "bundled scopes are granted");

    world.bridge.install(&world.id, revision(), capabilities.clone());
    assert_eq!(world.bridge.origin(&world.id), Origin::User);
    world.enable();
    world.grant("storage");
    assert_eq!(world.first(""), "none");
    assert_eq!(world.first("copied"), "copied");

    world.bridge.install_bundled(&world.id, bundled, capabilities);
    assert_eq!(world.first(""), "shipped");
    world.bridge.delete_data(&world.id, Origin::User).unwrap();
    assert_eq!(world.first(""), "shipped", "only the copy's data went");
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
    assert_eq!(rows[0].title, "revoked");
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
                 report(() => storage.put(undefined)),
                 report(() => storage.put('x'.repeat(1024 * 1024))),
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
        "invalid undeclared invalid invalid invalid invalid invalid invalid too-large out-of-scope invalid"
    );
    assert!(world.http.requests.lock().unwrap().is_empty());

    // Straight at the session, as a subverted helper might send them.
    let call = world.bridge.call(&world.id, Trigger::Query);
    let ask = |op: &str, args: &[u8]| -> serde_json::Value {
        serde_json::from_slice(&call.services.call(
            ASKED,
            op,
            args,
            Instant::now() + Duration::from_secs(1),
            &Default::default(),
        ))
        .unwrap()
    };
    for (op, args) in [
        ("storage.get", &b"not json"[..]),
        ("storage.get", br#"{"id":"000000000000000000000000","extra":1}"#),
        ("storage.get", br#"{"id":7}"#),
        ("storage.list", br#"{"before":"zz"}"#),
        ("storage.readBlob", br#"{"handle":"../key.check"}"#),
        ("storage.put", br#"{"data":1,"blobs":["***"]}"#),
        ("files.read", br#"{"scope":"notes","path":"a\u0000b"}"#),
        ("exec.run", br#"{"profile":"../bin/sh"}"#),
        ("dictionary.lookup", br#"{"word":"\u0007"}"#),
        ("calendar.events", br#"{"from":1,"to":0}"#),
        ("windows.focused", br#"{"pid":1}"#),
        ("context.frontmost", br#"{"pid":1}"#),
        ("context.document", br#"{"path":"/etc/passwd"}"#),
        ("windows.setFrame", br#"{"x":0,"y":0,"width":10}"#),
        (
            "windows.setFrame",
            br#"{"x":0,"y":0,"width":10,"height":10,"pid":1}"#,
        ),
        ("windows.setFrame", br#"{"x":"0","y":0,"width":10,"height":10}"#),
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
fn a_call_opens_only_the_calendar_links_its_own_reads_returned() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "calendar": { "access": ["read"], "pastDays": 0, "futureDays": 2 } }),
        &guest("export const search = () => [];"),
    );
    world.enable();
    world
        .bridge
        .set_os_access(OsPermission::Calendars, OsAccess::Allowed);
    let link = "ical://ekevent/standup-43200000?method=show&options=more";
    let read = |call: &Call| -> serde_json::Value {
        serde_json::from_slice(&call.services.call(
            ASKED,
            "calendar.events",
            br#"{"from":0,"to":86400000}"#,
            Instant::now() + Duration::from_secs(1),
            &Default::default(),
        ))
        .unwrap()
    };

    let refused = world.bridge.call(&world.id, Trigger::Query);
    assert_eq!(read(&refused)["error"]["code"], "needs-consent");
    assert!(!refused.services.opens(link));

    world.grant("calendar");
    let granted = world.bridge.call(&world.id, Trigger::Query);
    assert_eq!(read(&granted)["ok"]["events"][0]["link"], link);
    assert!(granted.services.opens(link));
    assert!(
        !granted
            .services
            .opens("ical://ekevent/other?method=show&options=more")
    );
    assert!(!world.bridge.call(&world.id, Trigger::Query).services.opens(link));
}

#[test]
fn a_latest_first_calendar_read_reaches_the_service() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "calendar": { "access": ["read"], "pastDays": 0, "futureDays": 2 } }),
        &guest("export const search = () => [];"),
    );
    world.enable();
    world.grant("calendar");
    world
        .bridge
        .set_os_access(OsPermission::Calendars, OsAccess::Allowed);
    let call = world.bridge.call(&world.id, Trigger::Query);
    let ask = |args: &[u8]| -> serde_json::Value {
        serde_json::from_slice(&call.services.call(
            ASKED,
            "calendar.events",
            args,
            Instant::now() + Duration::from_secs(1),
            &Default::default(),
        ))
        .unwrap()
    };
    assert!(ask(br#"{"from":0,"to":86400000,"order":"latest"}"#)["ok"].is_object());
    assert_eq!(
        ask(br#"{"from":0,"to":86400000,"order":"newest"}"#)["error"]["code"],
        "invalid"
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
fn calculations_need_the_grant_and_a_single_bounded_line() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "calculator": ["evaluate"] }),
        &guest(
            "export async function search(query) {
               return [{ title: await report(() => calculator.evaluate(query === 'long' ? '1+'.repeat(300) : query).then((answer) => answer?.result ?? 'none')) }];
             }",
        ),
    );
    world.enable();
    assert_eq!(world.first("1+1"), "needs-consent");
    world.grant("calculator");
    assert_eq!(world.first("1+1"), r#"ok "2""#);
    assert_eq!(world.first("2+2"), r#"ok "none""#);
    assert_eq!(world.first("long"), "invalid");
    assert_eq!(world.first("1\n+1"), "invalid");
}

#[test]
fn clipboard_changes_reach_the_handler_while_the_package_may_watch() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = Arc::new(world(
        serde_json::json!({ "clipboard": ["watch"], "storage": { "quotaBytes": 4096 } }),
        &guest(
            "export const search = () => [];
             export const events = { async clipboard(change) { await storage.put(change.text); } };",
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
                &serde_json::json!({}),
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
    wait_for(|| world.stored().first() == Some(&serde_json::json!("burst 19")));
    let mut seen = world.stored();
    seen.reverse();
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

#[test]
fn only_a_handle_the_watch_gave_puts_a_file_on_the_clipboard() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "clipboard": ["write"], "storage": { "quotaBytes": 4096 } }),
        &guest(
            "export const search = () => [];
             export const actions = {
               async forge({ path }) {
                 const { id } = await storage.put({}, { key: `dupin clipboard file\\u0000${path}` });
                 return { type: 'toast', text: await report(() => clipboard.write({ type: 'files', files: [{ path, handle: id }] })) };
               },
               async write({ path, handle }) {
                 return { type: 'toast', text: await report(() => clipboard.write({ type: 'files', files: [{ path, handle }] })) };
               },
             };",
        ),
    );
    world.enable();
    world.grant("clipboard:write");
    world.grant("storage");
    let secret = world.folder.path().join("id_ed25519");
    std::fs::write(&secret, "key").unwrap();
    let path = secret.to_string_lossy().into_owned();
    let toast = |name: &str, data: serde_json::Value| match world.act(name, data).unwrap().effect {
        Some(ActionEffect::Toast { text }) => text,
        other => panic!("{other:?}"),
    };

    assert_eq!(
        toast("forge", serde_json::json!({ "path": path })),
        "out-of-scope"
    );
    assert_eq!(*world.clipboard.text.lock().unwrap(), None);

    let handle = world
        .bridge
        .session(&world.id, Trigger::Background)
        .and_then(|session| session.file_handle(&path))
        .unwrap();
    assert_eq!(
        toast("write", serde_json::json!({ "path": path, "handle": handle })),
        "ok undefined"
    );
    assert_eq!(*world.clipboard.text.lock().unwrap(), Some(path));
}

#[test]
fn snippets_change_only_from_a_chosen_action_with_the_grant() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "snippets": ["write"] }),
        &guest(
            "export async function search(query) {
               return [{ title: await report(() => snippets.save({ keyword: query, text: 'x' })) }];
             }
             export const actions = {
               async save({ keyword, text, replaces }) {
                 return { type: 'toast', text: await report(() => snippets.save({ keyword, name: 'N', text }, { replaces })) };
               },
               async remove({ keyword }) {
                 return { type: 'toast', text: await report(() => snippets.delete(keyword)) };
               },
               async forge() {
                 return { type: 'toast', text: await report(() => snippets.save({ keyword: 'k', text: 7 })) };
               },
             };",
        ),
    );
    world.enable();
    let toast = |name: &str, data: serde_json::Value| match world.act(name, data).unwrap().effect {
        Some(ActionEffect::Toast { text }) => text,
        other => panic!("{other:?}"),
    };
    let sig = serde_json::json!({ "keyword": "sig", "text": "Best wishes" });
    assert_eq!(toast("save", sig.clone()), "needs-consent");
    world.grant("snippets");
    assert_eq!(world.first("typed"), "needs-activation", "a search never writes");
    assert!(world.snippets.0.lock().unwrap().is_empty());
    assert_eq!(toast("save", sig), "ok {}");
    assert_eq!(
        toast("save", serde_json::json!({ "keyword": "two words", "text": "x" })),
        r#"ok {"keyword":"A keyword has no spaces"}"#
    );
    assert_eq!(
        toast(
            "save",
            serde_json::json!({ "keyword": "sign", "text": "Cheers", "replaces": "sig" })
        ),
        "ok {}"
    );
    assert_eq!(
        world.snippets.0.lock().unwrap().keys().collect::<Vec<_>>(),
        ["sign"]
    );
    assert_eq!(
        toast(
            "save",
            serde_json::json!({ "keyword": "k", "text": "x".repeat(200 * 1024) })
        ),
        "invalid"
    );
    assert_eq!(toast("forge", serde_json::Value::Null), "invalid");
    assert_eq!(
        toast("remove", serde_json::json!({ "keyword": "sign" })),
        "ok true"
    );
    assert_eq!(
        toast("remove", serde_json::json!({ "keyword": "sign" })),
        "ok false"
    );
    world.bridge.revoke(&world.id, &ScopeKey::Snippets).unwrap();
    assert_eq!(
        toast("save", serde_json::json!({ "keyword": "late", "text": "x" })),
        "revoked"
    );
    assert!(world.snippets.0.lock().unwrap().is_empty());
}

fn wait_for(done: impl Fn() -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < until, "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_choice_that_cannot_be_saved_changes_nothing() {
    let folder = tempfile::tempdir().unwrap();
    let local = MemoryLocal::default();
    let bridge = Bridge::open(
        &folder.path().join("base.toml"),
        Box::new(local.clone()),
        Services::none(),
    );
    bridge.open_grants();
    let id = PackageId::new("weather").unwrap();
    bridge.install(&id, revision(), Capabilities::default());
    local.set_unwritable(Some("the disk is full"));
    assert!(matches!(bridge.set_enabled(&id, true), Err(SaveError::Store(_))));
    assert!(!bridge.is_enabled(&id));
    assert!(!bridge.is_turned_off(&id));
}

#[test]
fn settings_hears_of_a_base_entry_set_aside_and_its_package_stays_off() {
    let folder = tempfile::tempdir().unwrap();
    let base = folder.path().join("base.toml");
    std::fs::write(
        &base,
        "version = 1\n[packages.weather]\ndeny = [\"dictionery\"]\n[packages.news]\nenabled = false\n",
    )
    .unwrap();
    let bridge = Bridge::open(&base, Box::new(MemoryLocal::default()), Services::none());
    bridge.open_grants();
    let [problem] = bridge.problems().try_into().unwrap();
    assert!(
        problem.contains("weather") && problem.contains("dictionery"),
        "{problem}"
    );
    let weather = PackageId::new("weather").unwrap();
    let news = PackageId::new("news").unwrap();
    for id in [&weather, &news] {
        bridge.install(id, revision(), Capabilities::default());
    }
    assert!(bridge.set_enabled(&weather, true).is_err());
    assert!(bridge.is_turned_off(&weather));
    assert!(bridge.is_turned_off(&news), "the valid entry still applies");

    std::fs::write(
        &base,
        "version = 1\n[packages.weather]\ndeny = [\"dictionary\"]\n",
    )
    .unwrap();
    bridge.reload();
    assert!(
        bridge.problems().is_empty(),
        "fixing the entry clears its problem"
    );
}

#[test]
fn an_unusable_base_withholds_bundled_grants_and_says_why() {
    let folder = tempfile::tempdir().unwrap();
    let base = folder.path().join("base.toml");
    let calendar = PackageId::new("bundled:calendar").unwrap();
    let capabilities = || {
        Capabilities::parse(&serde_json::json!({
            "calendar": { "access": ["read"], "pastDays": 30, "futureDays": 90 }
        }))
        .unwrap()
    };
    let open = || {
        let bridge = Bridge::open(&base, Box::new(MemoryLocal::default()), Services::none());
        bridge.open_grants();
        bridge.set_bundled(&calendar, revision(), true);
        bridge.install_bundled(&calendar, revision(), capabilities());
        bridge
    };
    let states = |bridge: &Bridge| -> Vec<_> {
        bridge
            .scopes(&calendar)
            .into_iter()
            .map(|entry| entry.state)
            .collect()
    };

    let missing = open();
    assert!(missing.problems().is_empty(), "a missing base is an empty one");
    assert_eq!(states(&missing), [ConsentState::Granted(GrantSource::Bundled)]);

    std::fs::write(&base, "version = 1\n[packages\n").unwrap();
    let bridge = open();
    let [problem] = bridge.problems().try_into().unwrap();
    assert!(
        problem.starts_with(&base.display().to_string()) && problem.contains("can't be used"),
        "{problem}"
    );
    assert!(bridge.is_enabled(&calendar));
    assert_eq!(states(&bridge), [ConsentState::Undecided]);

    std::fs::write(&base, "version = 1\n").unwrap();
    bridge.reload();
    assert!(bridge.problems().is_empty());
    assert_eq!(
        states(&bridge),
        [ConsentState::Granted(GrantSource::Bundled)],
        "fixing the file restores what the build ships granted"
    );
}

#[test]
fn unreadable_grants_grant_nothing_are_never_overwritten_and_are_read_once_they_can_be() {
    let folder = tempfile::tempdir().unwrap();
    let id = PackageId::new("weather").unwrap();
    let local = MemoryLocal::default();
    let saved = serde_json::json!({ "package": "weather", "enabled": false });
    local.insert(id.clone(), saved.clone());
    local.set_unreadable(Some("the Keychain is locked"));
    let bridge = Bridge::open(
        &folder.path().join("base.toml"),
        Box::new(local.clone()),
        Services::none(),
    );
    bridge.open_grants();
    let [problem] = bridge.problems().try_into().unwrap();
    assert!(problem.contains("the Keychain is locked"), "{problem}");
    bridge.install(
        &id,
        revision(),
        Capabilities::parse(&serde_json::json!({ "dictionary": ["lookup"] })).unwrap(),
    );
    assert!(matches!(
        bridge.set_enabled(&id, true),
        Err(SaveError::Refused(BrokerError::GrantsUnreadable))
    ));
    assert!(matches!(
        bridge.forget_grants(&id),
        Err(SaveError::Refused(BrokerError::GrantsUnreadable))
    ));
    assert_eq!(local.records()[&id], saved, "what is stored is never overwritten");

    assert!(!bridge.grants_read());
    local.set_unreadable(None);
    bridge.open_grants();
    assert!(bridge.grants_read());
    bridge.set_enabled(&id, true).unwrap();
    assert!(bridge.problems().is_empty(), "reading them clears the problem");
    assert!(bridge.is_enabled(&id));

    let invalid = MemoryLocal::default();
    invalid.insert(
        id.clone(),
        serde_json::json!({ "package": "weather", "surprise": 1 }),
    );
    let bridge = Bridge::open(
        &folder.path().join("base.toml"),
        Box::new(invalid.clone()),
        Services::none(),
    );
    bridge.open_grants();
    bridge.install(&id, revision(), Capabilities::default());
    assert_eq!(bridge.problems().len(), 1);
    assert!(bridge.set_enabled(&id, true).is_err());
    assert_eq!(
        invalid.records()[&id],
        serde_json::json!({ "package": "weather", "surprise": 1 })
    );
}

/// A bridge over a base file in `folder` and the grants in `local`, with `weather` declaring
/// the dictionary and `notes` declaring nothing, both on and `weather`'s dictionary allowed.
fn granted(folder: &Path, local: &MemoryLocal) -> (Bridge, PackageId, PackageId, Arc<Mutex<Vec<Notice>>>) {
    let bridge = Bridge::open(
        &folder.join("base.toml"),
        Box::new(local.clone()),
        Services::none(),
    );
    bridge.open_grants();
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
fn forgotten_grants_stay_forgotten() {
    let folder = tempfile::tempdir().unwrap();
    let local = MemoryLocal::default();
    let (bridge, weather, notes, notices) = granted(folder.path(), &local);
    let call = bridge.call(&weather, Trigger::Activation);

    // What `dupin ext remove weather` asks the running app to do.
    bridge.forget_grants(&weather).unwrap();
    assert!(!local.records().contains_key(&weather));
    assert!(!bridge.is_current(&weather, call.generation));

    // Any save of the app's own must not write the removed grants back.
    bridge.set_enabled(&notes, false).unwrap();
    let saved = local.records();
    assert!(!saved.contains_key(&weather), "{saved:?}");
    assert!(saved.contains_key(&notes), "{saved:?}");
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
    let (bridge, weather, notes, notices) = granted(folder.path(), &MemoryLocal::default());

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

#[test]
fn an_effect_dupin_performs_for_a_package_is_decided_like_its_own_requests() {
    let bridge = Bridge::new(
        Broker::new(Base::default(), Local::default()),
        None,
        Services::none(),
    );
    let id = PackageId::new("emoji").unwrap();
    let paste = Request::Clipboard(ClipboardAccess::Paste);
    let declares = |value| Capabilities::parse(&value).unwrap();
    let _ = bridge.install(&id, revision(), declares(serde_json::json!({})));
    bridge.set_enabled(&id, true).unwrap();
    let refused = bridge.authorize(&id, &paste, Trigger::Activation).unwrap_err();
    assert_eq!(refused.code(), "undeclared");

    let _ = bridge.install(
        &id,
        revision(),
        declares(serde_json::json!({ "clipboard": ["paste"] })),
    );
    let refused = bridge.authorize(&id, &paste, Trigger::Activation).unwrap_err();
    assert_eq!(refused.code(), "needs-consent");
    let pending = bridge.pending(Some(&id));
    assert_eq!(pending[0].key().to_string(), "clipboard:paste");
    bridge.allow(&id, &revision(), &pending[0].scope, None).unwrap();
    assert_eq!(
        bridge.authorize(&id, &paste, Trigger::Query).unwrap_err().code(),
        "needs-activation"
    );
    let refused = bridge.authorize(&id, &paste, Trigger::Activation).unwrap_err();
    assert_eq!(refused.code(), "needs-consent");
    assert_eq!(
        bridge.pending(Some(&id))[0].permission,
        Some(OsPermission::Accessibility)
    );

    bridge.set_os_access(OsPermission::Accessibility, OsAccess::Allowed);
    let permit = bridge.authorize(&id, &paste, Trigger::Activation).unwrap();
    assert!(bridge.validate(&permit));
    bridge
        .revoke(&id, &ScopeKey::Clipboard(ClipboardAccess::Paste))
        .unwrap();
    assert!(
        !bridge.validate(&permit),
        "a revocation reaches a paste not yet made"
    );
    let missing = PackageId::new("missing").unwrap();
    assert_eq!(
        bridge
            .authorize(&missing, &paste, Trigger::Activation)
            .unwrap_err()
            .code(),
        "not-installed"
    );
}

#[test]
fn a_row_draws_only_images_from_its_own_package() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let capabilities = serde_json::json!({ "storage": { "quotaBytes": 4096 } });
    let world = world(
        capabilities.clone(),
        &guest(
            "export async function search() {
               const stored = await storage.put({}, { blobs: [new Uint8Array([1, 2, 3])] });
               return [{ title: stored.blobs[0].handle }];
             }",
        ),
    );
    let draw = |id: &PackageId, image: ImageRef<'_>| {
        world.bridge.read_image(id, &image, |data| match data {
            ImageData::Bytes(bytes) => format!("bytes {bytes:?}"),
            ImageData::File(path) => format!("file {}", path.display()),
        })
    };
    world.enable();
    let refused = draw(&world.id, ImageRef::Blob(&"0".repeat(64))).unwrap_err();
    assert_eq!(refused.code(), "needs-consent");
    assert!(
        world.bridge.pending(Some(&world.id)).is_empty(),
        "drawing a row never asks for consent"
    );

    world.grant("storage");
    let handle = world.first("");
    assert_eq!(
        draw(&world.id, ImageRef::Blob(&handle)).unwrap(),
        "bytes [1, 2, 3]"
    );

    let other = PackageId::new("other").unwrap();
    world
        .bridge
        .install(&other, revision(), Capabilities::parse(&capabilities).unwrap());
    world.bridge.set_enabled(&other, true).unwrap();
    let scope = world.bridge.scopes(&other).remove(0).scope;
    world.bridge.allow(&other, &revision(), &scope, None).unwrap();
    assert_eq!(
        draw(&other, ImageRef::Blob(&handle)).unwrap_err().code(),
        "not-found",
        "another package's handle names nothing in its own storage"
    );
    assert_eq!(
        draw(&world.id, ImageRef::Blob("../key.check"))
            .unwrap_err()
            .code(),
        "invalid"
    );

    let path = world.folder.path().join("Screenshot.png");
    let path = path.to_string_lossy().into_owned();
    let handle_of = |id: &PackageId| {
        world
            .bridge
            .session(id, Trigger::Background)
            .and_then(|session| session.file_handle(&path))
            .unwrap()
    };
    let delivered = handle_of(&world.id);
    assert_eq!(
        draw(
            &world.id,
            ImageRef::File {
                path: &path,
                handle: &delivered
            }
        )
        .unwrap(),
        format!("file {path}")
    );
    let others = handle_of(&other);
    for (id, handle) in [(&other, delivered.as_str()), (&world.id, others.as_str())] {
        assert_eq!(
            draw(id, ImageRef::File { path: &path, handle })
                .unwrap_err()
                .code(),
            "out-of-scope"
        );
    }
    assert_eq!(
        draw(&PackageId::new("missing").unwrap(), ImageRef::Blob(&handle))
            .unwrap_err()
            .code(),
        "not-installed"
    );
}

#[test]
fn context_reaches_a_search_only_scope_by_scope_and_only_while_allowed() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "context": ["frontmost"] }),
        &guest(
            "export async function search() {
               return [{ title: `${await report(() => context.frontmost())} ${await report(() => context.document())}` }];
             }",
        ),
    );
    world.enable();
    assert_eq!(
        world.first(""),
        "needs-consent undeclared",
        "nothing reaches the package until it is allowed, and an undeclared scope never does"
    );
    world.grant("context:frontmost");
    assert_eq!(
        world.first(""),
        r#"ok {"bundleId":"com.example.editor","name":"Editor","pid":4242} undeclared"#,
        "while typing, with no macOS access of its own, and never the document it didn't declare"
    );
    world
        .bridge
        .revoke(&world.id, &"context:frontmost".parse().unwrap())
        .unwrap();
    assert_eq!(world.first(""), "revoked undeclared");

    let world = world_with_document();
    world.enable();
    world.grant("context:document");
    assert_eq!(
        world.first(""),
        r#"needs-consent ok {"path":"/Users/me/notes.md"}"#,
        "each scope is allowed on its own"
    );
}

fn world_with_document() -> World {
    world(
        serde_json::json!({ "context": ["frontmost", "document"] }),
        &guest(
            "export async function search() {
               return [{ title: `${await report(() => context.frontmost())} ${await report(() => context.document())}` }];
             }",
        ),
    )
}

#[test]
fn a_search_may_read_windows_but_only_a_chosen_action_moves_one() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "windows": ["read", "arrange"] }),
        &guest(
            "const half = async () => {
               const [display] = await windows.displays();
               const focused = await windows.focused();
               const { x, y, width, height } = display.visibleFrame;
               const frame = await windows.setFrame({ x, y, width: width / 2, height });
               return `${focused.app.name} ${frame.width}x${frame.height}`;
             };
             export async function search() {
               return [{ title: `${await report(() => windows.focused().then((w) => w.frame.width))} ${await report(half)}` }];
             }
             export const actions = {
               async arrange() { return { type: 'toast', text: await report(half) }; },
             };",
        ),
    );
    world.enable();
    world.grant("windows:read");
    world.grant("windows:arrange");
    assert_eq!(
        world.first(""),
        "needs-consent needs-consent",
        "nothing reaches a window until macOS allows Accessibility"
    );
    world
        .bridge
        .set_os_access(OsPermission::Accessibility, OsAccess::Allowed);
    assert_eq!(world.first(""), "ok 300 needs-activation");
    assert!(world.windows.moves.lock().unwrap().is_empty());
    assert_eq!(world.toast("arrange"), "ok \"Notes 720x875\"");
    assert_eq!(
        *world.windows.moves.lock().unwrap(),
        [dupin_host_services::windows::Frame::new(0.0, 25.0, 720.0, 875.0)]
    );
}

#[test]
fn a_declared_folder_is_bound_without_a_pick_and_a_missing_one_asks_nothing() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "files": {
            "code": { "kind": "directory", "access": ["list"], "path": "~/code" },
            "gone": { "kind": "directory", "access": ["list"], "path": "~/gone" }
        } }),
        &guest(
            "export async function search() {
               return [{ title: (await Promise.all([
                 report(() => files.root('code')),
                 report(() => files.list('code').then((entries) => entries.map((entry) => entry.name))),
                 report(() => files.root('gone')),
                 report(() => files.list('gone')),
               ])).join(' | ') }];
             }",
        ),
    );
    let home = world.folder.path().join("home");
    std::fs::create_dir_all(home.join("code/dupin")).unwrap();
    world.bridge.set_home(Some(home.clone()));
    world.enable();
    assert_eq!(
        world.first(""),
        "needs-consent | needs-consent | not-found | not-found"
    );
    let pending = world.bridge.pending(Some(&world.id));
    assert_eq!(
        pending.iter().map(ConsentRequest::key).collect::<Vec<_>>(),
        ["files:code".parse::<ScopeKey>().unwrap()],
        "a folder the Mac doesn't have is never asked about"
    );
    assert!(!pending[0].needs_pick());
    assert_eq!(pending[0].description, "list the folder ~/code");
    world
        .bridge
        .allow(&world.id, &pending[0].revision, &pending[0].scope, None)
        .unwrap();
    let root = home.join("code");
    assert_eq!(
        world.first(""),
        format!(
            "ok {} | ok [\"dupin\"] | not-found | not-found",
            serde_json::Value::from(root.to_str().unwrap())
        )
    );
    world.bridge.show_root_as(root, "/Users/dupin/code".into());
    assert!(world.first("").starts_with("ok \"/Users/dupin/code\" |"));
}

#[test]
fn the_root_of_a_scope_needs_its_grant() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "files": { "notes": { "kind": "directory", "access": ["read"] } } }),
        &guest(
            "export async function search() { return [{ title: await report(() => files.root('notes')) }]; }",
        ),
    );
    world.enable();
    assert_eq!(world.first(""), "needs-consent");
    world.grant("files:notes");
    assert_eq!(
        world.first(""),
        format!(
            "ok {}",
            serde_json::Value::from(world.folder.path().to_str().unwrap())
        )
    );
    world
        .bridge
        .revoke(&world.id, &"files:notes".parse().unwrap())
        .unwrap();
    assert_eq!(world.first(""), "revoked");
}

#[test]
fn only_a_background_profile_runs_while_typing_and_never_outlives_its_call() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "exec": {
            "status": { "program": "/usr/bin/git", "args": ["-C", "{path}", "status"], "output": true, "background": true },
            "push": { "program": "/usr/bin/git", "args": ["-C", "{path}", "push"] }
        } }),
        &guest(
            "export async function search() {
               return [{ title: (await Promise.all([
                 report(() => exec.run('status', { path: '/repo' }).then((result) => result.stdout.trim())),
                 report(() => exec.run('push', { path: '/repo' })),
               ])).join(' | ') }];
             }",
        ),
    );
    world.enable();
    for key in ["exec:status", "exec:push"] {
        let key: ScopeKey = key.parse().unwrap();
        let scope = world
            .bridge
            .scopes(&world.id)
            .into_iter()
            .find(|entry| entry.scope.key() == key)
            .unwrap()
            .scope;
        assert!(!scope.needs_pick(), "the program is the one the package names");
        world.bridge.allow(&world.id, &revision(), &scope, None).unwrap();
    }
    assert_eq!(world.first(""), "ok \"clean\" | needs-activation");
    let launches = world.exec.launches.lock().unwrap();
    assert_eq!(launches.len(), 1);
    assert_eq!(
        launches[0].0,
        dupin_host_services::exec::Target::Executable("/usr/bin/git".into())
    );
    assert!(
        launches[0].1 < Duration::from_secs(5),
        "the command times out before the call's deadline: {:?}",
        launches[0].1
    );
}

/// When a request is held back from the bridge or from its guest, relative to its run's end.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hold {
    /// The consent request reaches the bridge only once its run has ended.
    UntilEnded,
    /// The consent request is decided while its run lives, and the fast failure that ends the
    /// run is released only then; its refusal reaches the guest only once the run has ended.
    UntilDecided,
}

/// A call's services with `storage.usage`, which needs consent, and `clipboard.read`, which
/// fails at once, put in the order `hold` names, so a guest that awaits both at once ends its
/// call in a known order. No timing decides it.
struct Ordered {
    inner: Arc<dyn HostServices>,
    hold: Hold,
    state: Mutex<Order>,
    changed: std::sync::Condvar,
}

#[derive(Default)]
struct Order {
    decided: bool,
    ended: bool,
    /// Whether the consent request has finished.
    finished: bool,
}

impl Ordered {
    fn new(inner: Arc<dyn HostServices>, hold: Hold) -> Arc<Self> {
        Arc::new(Self {
            inner,
            hold,
            state: Mutex::default(),
            changed: std::sync::Condvar::new(),
        })
    }

    fn wait(&self, until: impl Fn(&Order) -> bool) -> std::sync::MutexGuard<'_, Order> {
        let state = self.state.lock().unwrap();
        let (state, timeout) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(10), |state| !until(state))
            .unwrap();
        assert!(
            !timeout.timed_out(),
            "the requests never reached the order the test sets"
        );
        state
    }

    fn update(&self, change: impl FnOnce(&mut Order)) {
        change(&mut self.state.lock().unwrap());
        self.changed.notify_all();
    }
}

impl HostServices for Ordered {
    fn call(
        &self,
        request: HostRequest,
        op: &str,
        args: &[u8],
        deadline: Instant,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Vec<u8> {
        let call = || self.inner.call(request, op, args, deadline, cancelled);
        match (op, self.hold) {
            ("storage.usage", Hold::UntilEnded) => {
                drop(self.wait(|order| order.ended));
                let reply = call();
                self.update(|order| order.finished = true);
                reply
            }
            ("storage.usage", Hold::UntilDecided) => {
                let reply = call();
                self.update(|order| order.decided = true);
                drop(self.wait(|order| order.ended));
                self.update(|order| order.finished = true);
                reply
            }
            ("clipboard.read", Hold::UntilDecided) => {
                drop(self.wait(|order| order.decided));
                call()
            }
            _ => call(),
        }
    }

    fn ended(&self, run: u64, answered: &[u32]) {
        self.inner.ended(run, answered);
        self.update(|order| order.ended = true);
    }
}

#[test]
fn a_request_still_running_when_its_call_ends_never_leaves_a_consent_prompt() {
    dupin_extension_host::skip_inside_a_sandbox!();
    let world = world(
        serde_json::json!({ "storage": { "quotaBytes": 1024 } }),
        &guest(
            "export async function search(query) {
               if (query === 'together') await Promise.all([clipboard.readText(), storage.usage()]);
               else { try { await storage.usage(); } catch {} }
               return [{ title: 'done' }];
             }",
        ),
    );
    world.enable();
    let notices = Arc::new(Mutex::new(Vec::new()));
    world.bridge.set_listener({
        let notices = notices.clone();
        move |notice| notices.lock().unwrap().push(notice)
    });
    for hold in [Hold::UntilEnded, Hold::UntilDecided] {
        let call = world.bridge.call(&world.id, Trigger::Query);
        let ordered = Ordered::new(call.services.clone(), hold);
        let services: Arc<dyn HostServices> = ordered.clone();
        let answer = world.host.search(
            &world.package,
            "together",
            10,
            Instant::now() + Duration::from_secs(5),
            &|| false,
            &services,
        );
        let Err(RunError::Failed(diagnostic)) = answer else {
            panic!("the undeclared request didn't end the call: {answer:?}");
        };
        assert!(diagnostic.message.contains("clipboard"), "{diagnostic:?}");
        drop(ordered.wait(|order| order.finished));
        assert!(
            world.bridge.pending(Some(&world.id)).is_empty(),
            "a refusal its call never saw is still asked"
        );
        let notified = notices.lock().unwrap().drain(..).collect::<Vec<_>>();
        let expected = match hold {
            Hold::UntilEnded => Vec::new(),
            // Asked while the call ran, then withdrawn when it ended.
            Hold::UntilDecided => vec![Notice::ConsentNeeded(world.id.clone()); 2],
        };
        assert_eq!(notified, expected);
    }

    // A refusal the guest took while its call ran still asks, after the call ends.
    assert_eq!(world.first("apart"), "done");
    let pending = world.bridge.pending(Some(&world.id));
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].key().to_string(), "storage");
}

#[test]
fn a_question_stands_once_something_that_finished_asking_asked_it() {
    let bridge = Bridge::new(
        Broker::new(Base::default(), Local::default()),
        None,
        Services::none(),
    );
    let id = PackageId::new("emoji").unwrap();
    let capabilities =
        Capabilities::parse(&serde_json::json!({ "storage": { "quotaBytes": 1024 } })).unwrap();
    let _ = bridge.install(&id, revision(), capabilities);
    let usage = |call: &Call, request: HostRequest, ended: bool| {
        let reply = call.services.call(
            request,
            "storage.usage",
            b"{}",
            Instant::now() + Duration::from_secs(1),
            &std::sync::atomic::AtomicBool::new(ended),
        );
        serde_json::from_slice::<serde_json::Value>(&reply).unwrap()["error"]["code"].clone()
    };
    let first = HostRequest { run: 1, call: 1 };
    let second = HostRequest { run: 2, call: 1 };

    // Off: the request asks to turn the package on, until its run ends unseen.
    let call = bridge.call(&id, Trigger::Query);
    assert_eq!(usage(&call, first, false), "disabled");
    assert!(bridge.asked_while_off(&id));
    call.services.ended(first.run, &[]);
    assert!(!bridge.asked_while_off(&id));
    // Once its run has ended a request asks nothing and reaches no service.
    assert_eq!(usage(&call, first, true), "cancelled");
    assert!(!bridge.asked_while_off(&id));

    // Asked by two runs: one whose guest took the refusal keeps it up after the other ends
    // unseen, whichever ends first.
    bridge.set_enabled(&id, true).unwrap();
    for ends in [[(first, true), (second, false)], [(second, false), (first, true)]] {
        let call = bridge.call(&id, Trigger::Query);
        assert_eq!(usage(&call, first, false), "needs-consent");
        assert_eq!(usage(&call, second, false), "needs-consent");
        for (request, seen) in ends {
            let answered = if seen { vec![request.call] } else { Vec::new() };
            call.services.ended(request.run, &answered);
        }
        assert_eq!(bridge.pending(Some(&id)).len(), 1);
        bridge.forget_grants(&id).unwrap();
        bridge.set_enabled(&id, true).unwrap();
    }

    // Dupin's own refusal keeps up what a run then asks unseen, and a run's unseen refusal
    // gives way to Dupin's own.
    let read = Request::Storage { op: StorageOp::Read };
    for dupin_first in [true, false] {
        let call = bridge.call(&id, Trigger::Query);
        if !dupin_first {
            assert_eq!(usage(&call, first, false), "needs-consent");
        }
        let refused = bridge.authorize(&id, &read, Trigger::Query).unwrap_err();
        assert_eq!(refused.code(), "needs-consent");
        if dupin_first {
            assert_eq!(usage(&call, first, false), "needs-consent");
        }
        call.services.ended(first.run, &[]);
        assert_eq!(bridge.pending(Some(&id)).len(), 1);
        bridge.forget_grants(&id).unwrap();
        bridge.set_enabled(&id, true).unwrap();
    }
}
