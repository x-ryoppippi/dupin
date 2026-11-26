//! Sync between Macs as a host service, and the usage history it carries. One worker owns the
//! vault, the engine and the `usage` namespace: it loads and saves this Mac's ranking log, runs
//! a full sync pass every five minutes, and answers `dupin sync` through the control socket. It
//! never runs on the UI thread or the per-keystroke path.

use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dupin_core::History;
use dupin_mac::keychain::{ICloudKeychain, VaultKeychain};
use dupin_storage::{DeviceId, Limits, NewRecord, Storage, Store, VaultKey};
use dupin_sync::{
    DeviceInfo, Engine, Folder, FolderOptions, Joined, KdfParams, LocalKeyStore, Passphrase, Report,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::control::{Call, SyncAction};

const FULL_SCAN_EVERY: Duration = Duration::from_secs(5 * 60);
/// How long after a write to a synced namespace its changes go out, so a burst of edits is one
/// export.
const EXPORT_AFTER: Duration = Duration::from_secs(2);
/// How long after the folder changes it is read: the watcher already lets events settle.
const IMPORT_AFTER: Duration = Duration::from_millis(500);
const STATE_FILE: &str = "sync.json";
/// The namespace usage history lives in, one record per device.
const USAGE: &str = "usage";
const USAGE_LIMITS: Limits = Limits {
    max_age: None,
    max_entries: 1000,
    max_total_bytes: 64 * 1024 * 1024,
    max_item_bytes: 8 * 1024 * 1024,
};

/// What this Mac joined, kept beside the storage root: which vault, which key epoch and, unless
/// `[sync] folder` names one, the folder. Not a secret; the key itself is in the Keychain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    folder: PathBuf,
    vault: String,
    epoch: u32,
    /// Sync was turned off; the other Macs are taking this one's files over.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    leaving: bool,
}

fn state_path() -> Option<PathBuf> {
    crate::store::data_dir().map(|dir| dir.join(STATE_FILE))
}

fn read_state() -> Option<State> {
    let bytes = std::fs::read(state_path()?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_state(state: Option<&State>) -> Result<(), String> {
    let path = state_path().ok_or("Dupin's data folder needs HOME")?;
    match state {
        None => match std::fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.to_string()),
            _ => Ok(()),
        },
        Some(state) => {
            let json = serde_json::to_vec_pretty(state).map_err(|error| error.to_string())?;
            write_private(&path, &json).map_err(|error| error.to_string())
        }
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let temp = dir.join(format!(".{STATE_FILE}.{}.tmp", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(&temp, path)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex16(text: &str) -> Option<[u8; 16]> {
    if text.len() != 32
        || !text
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return None;
    }
    let mut out = [0u8; 16];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
}

/// What the UI learns about usage history.
pub enum UsageUpdate {
    /// This Mac's saved log, which picks made before it loaded add to, and the other Macs'.
    Loaded {
        own: History,
        others: History,
    },
    /// This Mac took a new device ID: its log starts empty, since its old one stays in the
    /// merge under the old ID.
    Restarted {
        others: History,
    },
    Others(History),
    /// A removed Mac's log, to fold into this Mac's.
    Absorb(History),
}

type Notify = Arc<dyn Fn(UsageUpdate) + Send + Sync>;

/// Where an answer goes: back down the control socket, or to Settings.
enum Reply {
    Call(Call),
    Callback(Box<dyn FnOnce(Result<Value, String>) + Send>),
}

impl Reply {
    fn send(self, result: Result<Value, String>) {
        match self {
            Reply::Call(call) => call.finish(result),
            Reply::Callback(callback) => callback(result),
        }
    }
}

enum Job {
    Config(dupin_core::config::Sync),
    Control(SyncAction, Reply),
    SaveUsage(History),
    /// A synced namespace was written on this Mac.
    Wrote,
    /// Something changed in the sync folder.
    FolderChanged,
}

/// The running worker's queue, so an owner can say it wrote without holding the service.
static JOBS: std::sync::Mutex<Option<mpsc::Sender<Job>>> = std::sync::Mutex::new(None);

/// An owner wrote to a namespace that syncs: export it shortly. Never blocks.
pub fn wrote() {
    if let Some(jobs) = &*JOBS.lock().unwrap_or_else(std::sync::PoisonError::into_inner) {
        let _ = jobs.send(Job::Wrote);
    }
}

/// The handle the UI keeps. Dropping it stops the worker.
pub struct SyncService {
    jobs: Option<mpsc::Sender<Job>>,
}

impl SyncService {
    /// Starts the worker with `[sync]` as configured now; `notify` hands usage history to the
    /// UI thread.
    pub fn start(
        config: &dupin_core::config::Sync,
        notify: impl Fn(UsageUpdate) + Send + Sync + 'static,
    ) -> Self {
        let Some(storage) = crate::extensions::access::shared_storage() else {
            return Self { jobs: None };
        };
        let (jobs, receive) = mpsc::channel();
        let _ = jobs.send(Job::Config(config.clone()));
        *JOBS.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(jobs.clone());
        let notify: Notify = Arc::new(notify);
        let own_jobs = jobs.clone();
        let spawned = std::thread::Builder::new()
            .name("dupin-sync".into())
            .spawn(move || Worker::new(storage, notify, own_jobs).run(receive));
        match spawned {
            Ok(_) => Self { jobs: Some(jobs) },
            Err(error) => {
                crate::log::warn(&format!("couldn't start sync: {error}"));
                Self { jobs: None }
            }
        }
    }

    /// For headless runs, which never touch the Keychain, storage or a sync folder.
    pub fn disabled() -> Self {
        Self { jobs: None }
    }

    pub fn apply(&self, config: &dupin_core::config::Sync) {
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(Job::Config(config.clone()));
        }
    }

    /// Saves this Mac's usage log; snapshots that pile up while one is written collapse.
    pub fn save_usage(&self, own: &History) {
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(Job::SaveUsage(own.clone()));
        }
    }

    pub fn control(&self, action: SyncAction, call: Call) {
        self.send(action, Reply::Call(call));
    }

    /// Asks the worker as `dupin sync` does, for Settings; `reply` runs on the worker.
    pub fn request(&self, action: SyncAction, reply: impl FnOnce(Result<Value, String>) + Send + 'static) {
        self.send(action, Reply::Callback(Box::new(reply)));
    }

    fn send(&self, action: SyncAction, reply: Reply) {
        match &self.jobs {
            Some(jobs) => {
                if let Err(mpsc::SendError(Job::Control(_, reply))) = jobs.send(Job::Control(action, reply)) {
                    reply.send(Err("sync stopped".into()));
                }
            }
            None => reply.send(Err("sync is not available in this build or session".into())),
        }
    }
}

/// Item IDs under the home folder are kept relative to it, so the same file on two Macs with
/// different user names is one item.
struct Home(Option<String>);

impl Home {
    fn new() -> Self {
        Self(std::env::var("HOME").ok().filter(|home| home.starts_with('/')))
    }

    fn prefix(&self) -> Option<String> {
        self.0
            .as_ref()
            .map(|home| format!("{}/", home.trim_end_matches('/')))
    }

    fn portable(&self, history: &History) -> History {
        let Some(prefix) = self.prefix() else {
            return history.clone();
        };
        history.map_ids(|id| match id.strip_prefix(&prefix) {
            Some(rest) => Some(format!("~/{rest}")),
            None => Some(id.to_owned()),
        })
    }

    fn local(&self, history: &History) -> History {
        let Some(prefix) = self.prefix() else {
            return history.clone();
        };
        history.map_ids(|id| match id.strip_prefix("~/") {
            Some(rest) => Some(format!("{prefix}{rest}")),
            None => Some(id.to_owned()),
        })
    }
}

/// Folds a removed Mac's usage log into this Mac's, through the UI, which owns this Mac's log.
struct UsageFold {
    notify: Notify,
}

impl dupin_sync::Fold for UsageFold {
    fn fold(&self, store: &Store, _own: DeviceId, leaving: DeviceId) -> Result<(), dupin_storage::Error> {
        if let Some(record) = store.get(&store.key_id(&leaving.to_string()))?
            && let Ok(history) = serde_json::from_value::<History>(record.data)
        {
            (self.notify)(UsageUpdate::Absorb(Home::new().local(&history)));
        }
        Ok(())
    }
}

struct Worker {
    storage: Arc<Storage>,
    notify: Notify,
    home: Home,
    config: dupin_core::config::Sync,
    state: Option<State>,
    vault: Option<VaultKey>,
    engine: Option<Engine>,
    usage: Option<Store>,
    others: History,
    last: Option<(u64, Report)>,
    problem: Option<String>,
    started: bool,
    jobs: mpsc::Sender<Job>,
    /// Watches the folder while the engine runs, so another Mac's changes are read at once.
    watcher: Option<dupin_mac::watcher::PathWatcher>,
    /// When the next pass is due before the five-minute full scan.
    due: Option<std::time::Instant>,
}

impl Worker {
    fn new(storage: Arc<Storage>, notify: Notify, jobs: mpsc::Sender<Job>) -> Self {
        Self {
            jobs,
            watcher: None,
            due: None,
            storage,
            notify,
            home: Home::new(),
            config: dupin_core::config::Sync::default(),
            state: read_state(),
            vault: None,
            engine: None,
            usage: None,
            others: History::default(),
            last: None,
            problem: None,
            started: false,
        }
    }

    /// Asks for a pass within `after`, keeping an earlier one already due.
    fn soon(&mut self, after: Duration) {
        let at = std::time::Instant::now() + after;
        self.due = Some(self.due.map_or(at, |due| due.min(at)));
    }

    fn run(mut self, jobs: mpsc::Receiver<Job>) {
        loop {
            let wait = self.due.map_or(FULL_SCAN_EVERY, |due| {
                due.saturating_duration_since(std::time::Instant::now())
            });
            match jobs.recv_timeout(wait) {
                Ok(Job::SaveUsage(mut own)) => {
                    // Later snapshots replace earlier ones; anything else waits its turn.
                    let mut after = Vec::new();
                    while let Ok(next) = jobs.try_recv() {
                        match next {
                            Job::SaveUsage(newer) => own = newer,
                            other => after.push(other),
                        }
                    }
                    self.save_usage(&own);
                    for job in after {
                        self.handle(job);
                    }
                }
                Ok(job) => self.handle(job),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    self.due = None;
                    self.pass();
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn handle(&mut self, job: Job) {
        match job {
            Job::Config(config) => {
                if !self.started || config != self.config {
                    self.config = config;
                    self.start();
                }
            }
            Job::Control(action, reply) => {
                let result = self.control(action);
                reply.send(result);
            }
            Job::SaveUsage(own) => self.save_usage(&own),
            Job::Wrote => self.soon(EXPORT_AFTER),
            Job::FolderChanged => self.soon(IMPORT_AFTER),
        }
    }

    /// Opens what the configuration and `sync.json` call for: the vault key from this Mac's
    /// own copy, which needs no folder, then the usage namespace sealed under it, then, when
    /// the folder is there, the engine.
    fn start(&mut self) {
        let first = !self.started;
        self.started = true;
        self.problem = None;
        self.engine = None;
        self.watcher = None;
        self.vault = None;
        if let Some(state) = self.state.clone().filter(|state| !state.leaving)
            && let Some(vault) = unhex16(&state.vault)
        {
            match VaultKeychain.load(vault, state.epoch) {
                Ok(Some(key)) => self.vault = VaultKey::new(vault, state.epoch, key).ok(),
                Ok(None) => {
                    self.problem = Some("this Mac's copy of the sync key is missing; join again".into())
                }
                Err(error) => self.problem = Some(error.to_string()),
            }
        }
        if let Some(vault) = &self.vault {
            self.storage.hold_vault_key(vault.clone());
        }
        crate::synced::set_switches(self.config.clone());
        crate::synced::set_vault(self.vault.clone());
        self.open_usage(first);
        self.pass();
    }

    /// Opens the usage namespace, at launch only when it or `history.json` exists, so a Mac
    /// with nothing recorded reads no key until the first pick.
    fn open_usage(&mut self, migrate: bool) {
        self.usage = None;
        let recorded = crate::store::usage_dir().is_some_and(|dir| dir.is_dir())
            || crate::store::legacy_history_path().is_some_and(|path| path.is_file());
        if migrate && self.vault.is_none() && !recorded {
            return;
        }
        let opened = match self.vault.as_ref().filter(|_| self.config.usage) {
            Some(vault) => self.storage.open_synced(USAGE, USAGE_LIMITS, vault),
            None => self.storage.open(USAGE, USAGE_LIMITS),
        };
        match opened {
            Ok(store) => self.usage = Some(store),
            Err(error) => {
                crate::log::warn(&format!("couldn't open the usage history: {error}"));
                return;
            }
        }
        if migrate {
            self.migrate_history_file();
        }
        let (own, others) = self.read_usage();
        self.others = others.clone();
        // Reopened under another key, the store holds what the UI has already saved, so only
        // the first open hands this Mac's log over.
        match migrate {
            true => (self.notify)(UsageUpdate::Loaded { own, others }),
            false => (self.notify)(UsageUpdate::Others(others)),
        }
    }

    /// `history.json` from before usage history moved into encrypted storage becomes this
    /// Mac's record, and the file goes once the record is stored.
    fn migrate_history_file(&mut self) {
        let Some(path) = crate::store::legacy_history_path() else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        match History::from_json(&text) {
            Ok(legacy) => {
                let (own, _) = self.read_usage();
                let merged = History::merged([&own, &legacy], now_millis() / 1000);
                if self.write_usage(&merged).is_ok() {
                    let _ = std::fs::remove_file(&path);
                }
            }
            Err(error) => crate::log::warn(&format!(
                "{} is invalid ({error}); it is left as it is",
                path.display()
            )),
        }
    }

    fn device_key(&self) -> Option<String> {
        self.storage.device_id().ok().map(|id| id.to_string())
    }

    /// This Mac's log and every other Mac's merged, with IDs back in this Mac's home folder.
    fn read_usage(&self) -> (History, History) {
        let (Some(store), Some(me)) = (&self.usage, self.device_key()) else {
            return (History::default(), History::default());
        };
        let own_id = store.key_id(&me);
        let mut own = History::default();
        let mut others = Vec::new();
        let mut cursor = None;
        while let Ok(page) = store.list(cursor.as_ref(), 200) {
            for record in page.records {
                let Ok(history) = serde_json::from_value::<History>(record.data) else {
                    continue;
                };
                if record.id == own_id {
                    own = history;
                } else {
                    others.push(history);
                }
            }
            match page.next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        let now = now_millis() / 1000;
        (
            self.home.local(&own),
            self.home.local(&History::merged(&others, now)),
        )
    }

    fn write_usage(&mut self, own: &History) -> Result<(), String> {
        if self.usage.is_none() {
            let opened = match self.vault.as_ref().filter(|_| self.config.usage) {
                Some(vault) => self.storage.open_synced(USAGE, USAGE_LIMITS, vault),
                None => self.storage.open(USAGE, USAGE_LIMITS),
            };
            self.usage = Some(opened.map_err(|error| error.to_string())?);
        }
        let (Some(store), Some(me)) = (&self.usage, self.device_key()) else {
            return Err("usage history isn't open".into());
        };
        let data = serde_json::to_value(self.home.portable(own)).map_err(|error| error.to_string())?;
        store
            .upsert(
                &me,
                NewRecord {
                    data,
                    blobs: Vec::new(),
                },
            )
            .map(drop)
            .map_err(|error| error.to_string())
    }

    fn save_usage(&mut self, own: &History) {
        match self.write_usage(own) {
            Ok(()) => self.soon(EXPORT_AFTER),
            Err(error) => crate::log::warn(&format!("couldn't save the usage history: {error}")),
        }
    }

    fn data_dir() -> Result<PathBuf, String> {
        crate::store::data_dir().ok_or_else(|| "Dupin's data folder needs HOME".into())
    }

    fn options(&self) -> FolderOptions {
        FolderOptions {
            allow_nfs: self.config.allow_nfs,
        }
    }

    /// The folder `[sync] folder` names, which wins, or the one chosen when sync was turned on.
    fn folder_path(&self, chosen: Option<&Path>) -> Option<PathBuf> {
        match self.config.folder.trim() {
            "" => chosen.map(Path::to_owned),
            configured => Some(expand_home(configured)),
        }
    }

    fn open_folder(&self, path: &Path) -> Result<Folder, String> {
        Folder::open(path, &Self::data_dir()?, self.options()).map_err(|error| error.to_string())
    }

    /// Builds the engine once the folder is reachable; until then local use goes on.
    fn connect(&mut self) {
        if self.engine.is_some() {
            return;
        }
        let Some(state) = self.state.clone() else {
            return;
        };
        let Some(vault) = unhex16(&state.vault) else {
            self.problem = Some("sync.json is damaged; turn sync off and on again".into());
            return;
        };
        let Some(path) = self.folder_path(Some(&state.folder)) else {
            return;
        };
        let result = self
            .open_folder(&path)
            .and_then(|folder| {
                dupin_sync::resume(vault, state.epoch, Some(&folder), &VaultKeychain)
                    .map(|joined| (folder, joined))
                    .map_err(|error| format!("folder unavailable: {error}"))
            })
            .and_then(|(folder, joined)| self.engine_for(folder, joined));
        match result {
            Ok(engine) => {
                self.watch(engine.folder_path());
                self.engine = Some(engine);
            }
            Err(error) => self.problem = Some(error),
        }
    }

    /// Watches the folder for another Mac's changes. Without a watcher the five-minute scan
    /// still finds them.
    fn watch(&mut self, folder: &Path) {
        let jobs = self.jobs.clone();
        match dupin_mac::watcher::PathWatcher::start_scoped(
            Vec::new(),
            vec![folder.to_owned()],
            Vec::new,
            move |_| {
                let _ = jobs.send(Job::FolderChanged);
            },
        ) {
            Ok(watcher) => self.watcher = Some(watcher),
            Err(error) => {
                self.watcher = None;
                crate::log::warn(&format!("couldn't watch the sync folder: {error}"));
            }
        }
    }

    fn engine_for(&self, folder: Folder, joined: Joined) -> Result<Engine, String> {
        let mut engine = Engine::new(
            folder,
            self.storage.clone(),
            joined.key,
            joined.header,
            DeviceInfo {
                name: dupin_mac::machine::computer_name().unwrap_or_else(|| "Mac".into()),
                platform: "macos".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            Arc::new(dupin_storage::SystemClock),
        )
        .map_err(|error| error.to_string())?;
        let root = engine.folder_path().to_owned();
        engine.on_dataless(move |relative| dupin_mac::machine::request_download(&root.join(relative)));
        Ok(engine)
    }

    fn pass(&mut self) {
        self.connect();
        let Some(engine) = &mut self.engine else {
            return;
        };
        let leaving = self.state.as_ref().is_some_and(|state| state.leaving);
        let fold = UsageFold {
            notify: self.notify.clone(),
        };
        let owned = crate::synced::open_stores();
        let mut namespaces: Vec<dupin_sync::Namespace<'_>> = owned
            .iter()
            .filter(|(name, _, _)| self.config.includes(name) || name.starts_with("ext-s-"))
            .map(|(name, mode, store)| dupin_sync::Namespace {
                name,
                store,
                mode: *mode,
                fold: None,
            })
            .collect();
        if let Some(store) = self.usage.as_ref().filter(|_| self.config.usage) {
            namespaces.push(dupin_sync::Namespace {
                name: USAGE,
                store,
                mode: dupin_sync::Mode::PerDevice,
                fold: Some(&fold),
            });
        }
        let all: Vec<&str> = crate::synced::SYNCED.iter().map(|(name, _)| *name).collect();
        let revoked = crate::synced::leaving_extensions();
        let mut switched_off: Vec<&str> = all
            .iter()
            .copied()
            .filter(|name| !self.config.includes(name))
            .collect();
        switched_off.extend(revoked.iter().map(String::as_str));
        let mut every: Vec<&str> = all.clone();
        every.extend(
            owned
                .iter()
                .map(|(name, _, _)| name.as_str())
                .filter(|name| name.starts_with("ext-s-")),
        );
        every.extend(revoked.iter().map(String::as_str));
        let result = match leaving {
            false => engine.run(&namespaces, &switched_off),
            true => engine.run(&[], &every),
        };
        drop(namespaces);
        drop(owned);
        match result {
            Ok(report) => {
                self.problem = None;
                for name in &report.changed {
                    if name != USAGE {
                        crate::synced::changed(name);
                    }
                }
                for name in &report.left {
                    crate::synced::left_extension(name);
                }
                let left = all.iter().all(|name| report.left.iter().any(|left| left == name));
                let reminted = report.reminted;
                self.last = Some((now_millis(), report));
                let (_, others) = self.read_usage();
                if leaving && left {
                    self.finish_leaving();
                } else if reminted {
                    self.others = others.clone();
                    (self.notify)(UsageUpdate::Restarted { others });
                } else if others != self.others {
                    self.others = others.clone();
                    (self.notify)(UsageUpdate::Others(others));
                }
            }
            Err(error) => self.problem = Some(error.to_string()),
        }
    }

    fn control(&mut self, action: SyncAction) -> Result<Value, String> {
        match action {
            SyncAction::Status {} => Ok(self.status()),
            SyncAction::On { folder, passphrase } => self.turn_on(folder, passphrase),
            SyncAction::Join { folder, passphrase } => self.join(folder, passphrase),
            SyncAction::Off {} => self.turn_off(),
            SyncAction::RemoveDevice { device } => {
                let engine = self
                    .engine
                    .as_mut()
                    .ok_or("sync is off or its folder is unavailable")?;
                let device = device.parse().map_err(|_| "not a device ID")?;
                engine.retire(device).map_err(|error| error.to_string())?;
                self.pass();
                Ok(self.status())
            }
            SyncAction::Now {} => {
                self.pass();
                Ok(self.status())
            }
        }
    }

    fn chosen_folder(&self, folder: Option<String>) -> Result<PathBuf, String> {
        let configured = self.folder_path(None);
        match (folder, configured) {
            (Some(asked), Some(configured)) if expand_home(&asked) != configured => Err(format!(
                "[sync] folder in config.toml names {}; change it there",
                configured.display()
            )),
            (Some(asked), _) => Ok(expand_home(&asked)),
            (None, Some(configured)) => Ok(configured),
            (None, None) => std::env::var_os("HOME")
                .and_then(|home| dupin_sync::icloud_drive_default(Path::new(&home)))
                .ok_or_else(|| "iCloud Drive isn't set up on this Mac; choose a folder with --folder".into()),
        }
    }

    fn turn_on(&mut self, folder: Option<String>, passphrase: Option<String>) -> Result<Value, String> {
        if self.state.is_some() {
            return Err("sync is already on, or still being turned off".into());
        }
        let path = self.chosen_folder(folder)?;
        let folder = self.open_folder(&path)?;
        let (passphrase, generated) = match passphrase {
            Some(text) => (Passphrase::chosen(&text).map_err(|weak| weak.to_string())?, false),
            None => (
                Passphrase::generate(&word_list()?).map_err(|error| error.to_string())?,
                true,
            ),
        };
        let shared = ICloudKeychain::available();
        let device = self.storage.device_id().map_err(|error| error.to_string())?;
        let joined = dupin_sync::create(
            &folder,
            device,
            &passphrase,
            KdfParams::DEFAULT,
            &VaultKeychain,
            shared
                .as_ref()
                .map(|keychain| keychain as &dyn dupin_sync::SharedKeyStore),
        )
        .map_err(|error| match error {
            dupin_sync::Error::Refused(dupin_sync::Refusal::HoldsAVault) => {
                "the folder already holds a sync vault; join it with `dupin sync join`".into()
            }
            other => other.to_string(),
        })?;
        self.joined(path, folder, joined)?;
        let mut status = self.status();
        if generated {
            status["passphrase"] = json!(passphrase.expose());
        }
        Ok(status)
    }

    fn join(&mut self, folder: Option<String>, passphrase: Option<String>) -> Result<Value, String> {
        if self.state.is_some() {
            return Err("sync is already on, or still being turned off".into());
        }
        let path = self.chosen_folder(folder)?;
        let folder = self.open_folder(&path)?;
        let passphrase = passphrase.as_deref().map(Passphrase::entered);
        let shared = ICloudKeychain::available();
        let joined = dupin_sync::join(
            &folder,
            passphrase.as_ref(),
            &VaultKeychain,
            shared
                .as_ref()
                .map(|keychain| keychain as &dyn dupin_sync::SharedKeyStore),
        )
        .map_err(|error| error.to_string())?;
        self.joined(path, folder, joined)?;
        Ok(self.status())
    }

    /// Records the vault, re-seals usage history under its key and syncs at once.
    fn joined(&mut self, path: PathBuf, folder: Folder, joined: Joined) -> Result<(), String> {
        let state = State {
            folder: path,
            vault: hex(&joined.key.id()),
            epoch: joined.key.epoch(),
            leaving: false,
        };
        write_state(Some(&state))?;
        self.state = Some(state);
        self.vault = Some(joined.key.clone());
        self.storage.hold_vault_key(joined.key.clone());
        crate::synced::set_vault(Some(joined.key.clone()));
        let engine = self.engine_for(folder, joined)?;
        self.watch(engine.folder_path());
        self.engine = Some(engine);
        self.open_usage(false);
        self.pass();
        Ok(())
    }

    /// This Mac leaves every namespace: usage history goes back under the data key at once,
    /// and its files stay in the folder until the other Macs have taken them over.
    fn turn_off(&mut self) -> Result<Value, String> {
        let Some(mut state) = self.state.clone() else {
            return Ok(self.status());
        };
        state.leaving = true;
        write_state(Some(&state))?;
        self.state = Some(state);
        self.vault = None;
        crate::synced::set_vault(None);
        self.open_usage(false);
        self.pass();
        if self.engine.is_none() {
            self.finish_leaving();
        }
        Ok(self.status())
    }

    fn finish_leaving(&mut self) {
        if let Err(error) = write_state(None) {
            self.problem = Some(error);
            return;
        }
        self.state = None;
        self.engine = None;
        self.watcher = None;
    }

    fn status(&self) -> Value {
        let report = self.last.as_ref().map(|(at, report)| {
            json!({
                "at": at,
                "exported": report.exported,
                "imported": report.imported,
                "removed": report.removed,
                "not_arrived": report.not_arrived,
                "unreadable": report.unreadable,
                "clock_ahead": report.clock_ahead.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "losing_vault": report.losing_vault,
                "newer_format": report.newer_format,
                "newer_epoch": report.newer_epoch,
                "reminted": report.reminted,
                "blocked_adoption": report.blocked_adoption.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "devices": report.devices.iter().map(|device| json!({
                    "id": device.id.to_string(),
                    "name": device.name,
                    "seen_at": device.seen_at,
                    "inactive": device.inactive,
                    "retired": device.retired,
                })).collect::<Vec<_>>(),
            })
        });
        json!({
            "on": self.state.as_ref().is_some_and(|state| !state.leaving),
            "leaving": self.state.as_ref().is_some_and(|state| state.leaving),
            "folder": self
                .state
                .as_ref()
                .and_then(|state| self.folder_path(Some(&state.folder)))
                .or_else(|| self.folder_path(None))
                .map(|path| path.display().to_string()),
            "vault": self.state.as_ref().map(|state| state.vault.clone()),
            "mode": self.engine.as_ref().map(|engine| match engine.key_mode() {
                dupin_sync::KeyMode::Keychain => "icloud-keychain",
                dupin_sync::KeyMode::Passphrase => "passphrase",
            }),
            "device": self.device_key(),
            "namespaces": match self.vault.is_some() {
                true => crate::synced::SYNCED
                    .iter()
                    .map(|(name, _)| *name)
                    .filter(|name| self.config.includes(name))
                    .collect(),
                false => Vec::new(),
            },
            "problem": self.problem,
            "last": report,
        })
    }
}

/// The EFF long word list, which ships as a file in Dupin.app's Resources since its licence
/// (CC BY 3.0 US) keeps it out of the binary; a build from the source tree reads the
/// repository's copy.
fn word_list() -> Result<dupin_sync::WordList, String> {
    const FILE: &str = "eff_large_wordlist.txt";
    let executable = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .map_err(|error| error.to_string())?;
    let path = match crate::extensions::bundled::resources(&executable) {
        Some(resources) => resources.join(FILE),
        None => Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../dupin-sync/data")
            .join(FILE),
    };
    let text = std::fs::read_to_string(&path).map_err(|error| {
        format!(
            "couldn't read the passphrase word list {}: {error}",
            path.display()
        )
    })?;
    dupin_sync::WordList::parse(&text)
}

fn expand_home(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_rejects_anything_else() {
        let id = [0xab; 16];
        assert_eq!(unhex16(&hex(&id)), Some(id));
        assert_eq!(unhex16("AB"), None);
        assert_eq!(unhex16(&"AB".repeat(16)), None);
    }

    #[test]
    fn a_source_build_finds_the_word_list() {
        assert!(word_list().is_ok());
    }

    #[test]
    fn a_home_relative_folder_expands() {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        assert_eq!(expand_home("~/Sync"), home.join("Sync"));
        assert_eq!(expand_home("/Volumes/Sync"), PathBuf::from("/Volumes/Sync"));
    }

    #[test]
    fn home_paths_travel_relative_to_home() {
        let history = History::from_json(
            r#"{"items":{"/Users/a/Documents/x.txt":{"count":1,"visits":[1],"queries":{}},
            "/Applications/Safari.app":{"count":1,"visits":[1],"queries":{}}}}"#,
        )
        .unwrap();
        let portable = Home(Some("/Users/a".into())).portable(&history);
        assert!(portable.to_json().contains("\"~/Documents/x.txt\""));
        let elsewhere = Home(Some("/Users/b/".into())).local(&portable).to_json();
        assert!(elsewhere.contains("\"/Users/b/Documents/x.txt\""), "{elsewhere}");
        assert!(elsewhere.contains("\"/Applications/Safari.app\""));
    }
}
