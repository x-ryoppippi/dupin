//! The anonymous daily usage ping (docs/features/telemetry.md).
//!
//! Only the build `scripts/package-app.nu` signs carries the collector's address: `nix build
//! .#dupin-release` compiles it in from `DUPIN_TELEMETRY_ENDPOINT`, and `.#dupin`, `cargo run`
//! and every other build have none, so they never send. Even with the address, a copy sends only
//! while it is signed with a Developer ID, as the updater requires.
//!
//! The ping leaves from a worker thread, at most once per UTC day, through curl, and is never
//! retried. Before the first one the user is told: by the first-run welcome, or the first time
//! the panel opens after an update brought telemetry. The launch that told them sends nothing.
//! `telemetry = false`, `DUPIN_TELEMETRY_DISABLED` or `DO_NOT_TRACK` turns it off at once, and
//! turning the setting off deletes the stored anonymous ID.

use std::io::Write;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dupin_storage::{Limits, NewRecord, Storage, Store};
use dupin_telemetry::{Channel, Ping, PingEvent};
use serde::{Deserialize, Serialize};

const ENDPOINT: Option<&str> = option_env!("DUPIN_TELEMETRY_ENDPOINT");
pub const DOCS_URL: &str = "https://dupin.app/docs/features/telemetry";
/// What the welcome, Settings and the panel say is sent.
pub const SENT: &str = "Once a day: Dupin's version and channel, the macOS version, the chip and a random ID. \
                        Nothing you search, open or set.";
/// The panel's notice for someone who never saw the welcome's.
pub const NOTICE: &str = "Dupin now sends one anonymous usage ping a day: its version and channel, the macOS \
                          version, the chip and a random ID, nothing you search or open. Settings → General \
                          turns it off.";
const DISABLE_VARS: [&str; 2] = ["DUPIN_TELEMETRY_DISABLED", "DO_NOT_TRACK"];

/// A namespace of its own, which never syncs: the anonymous ID belongs to this Mac.
const NAMESPACE: &str = "telemetry";
const RECORD: &str = "state";
const LIMITS: Limits = Limits {
    max_age: None,
    max_entries: 4,
    max_total_bytes: 64 * 1024,
    max_item_bytes: 16 * 1024,
};

/// Long enough after launch not to compete with the first app scan and the panel's warm-up.
const FIRST_CHECK: Duration = Duration::from_secs(60);
/// A running launcher sends within this long of a UTC day starting.
const RECHECK: Duration = Duration::from_secs(60 * 60);
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
const SECONDS_PER_DAY: u64 = 86_400;

/// Why no ping is sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Off {
    /// `DUPIN_TELEMETRY_DISABLED` or `DO_NOT_TRACK`.
    Env(&'static str),
    /// `telemetry = false`.
    Setting,
    /// No collector address: a Nix, `cargo run` or other unofficial build.
    Unofficial,
    /// The address, but no Developer ID signature.
    Unsigned,
}

impl Off {
    pub fn describe(self) -> String {
        match self {
            Off::Env(name) => format!("Off because {name} is set"),
            Off::Setting => "Off in Settings or config.toml".into(),
            Off::Unofficial | Off::Unsigned => {
                "Never sent from this build; only signed releases send it".into()
            }
        }
    }
}

/// What is stored between launches, in encrypted storage.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct State {
    #[serde(skip_serializing_if = "Option::is_none")]
    anonymous_id: Option<String>,
    /// When the user was first told, in Unix seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    noticed_at: Option<u64>,
    /// The UTC day, counted from the epoch, of the last ping.
    #[serde(skip_serializing_if = "Option::is_none")]
    last_sent_day: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
enum Plan {
    /// Tell the user first.
    Notice,
    /// Store what changed and send nothing.
    Save,
    Nothing,
    Send,
}

/// The launch that told the user sends nothing, so they can turn it off before the first ping;
/// after that, one a UTC day.
fn plan(state: &mut State, now: u64, noticed_here: bool) -> Plan {
    match state.noticed_at {
        None if noticed_here => {
            state.noticed_at = Some(now);
            Plan::Save
        }
        None => Plan::Notice,
        Some(_) if noticed_here => Plan::Nothing,
        Some(_) => {
            let today = now / SECONDS_PER_DAY;
            if state.last_sent_day == Some(today) {
                return Plan::Nothing;
            }
            state.last_sent_day = Some(today);
            Plan::Send
        }
    }
}

fn truthy(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        let value = value.trim();
        !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false")
    })
}

fn env_off(lookup: impl Fn(&str) -> Option<String>) -> Option<&'static str> {
    DISABLE_VARS
        .into_iter()
        .find(|name| truthy(lookup(name).as_deref()))
}

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// Why this process sends nothing with `telemetry` set to `enabled`, or `None` when it sends.
pub fn off(enabled: bool) -> Option<Off> {
    let unsigned = get().is_some_and(|telemetry| telemetry.shared.unsigned.load(Ordering::Relaxed));
    decide(env_off(env_var), enabled, ENDPOINT.is_some(), unsigned)
}

fn decide(env: Option<&'static str>, enabled: bool, endpoint: bool, unsigned: bool) -> Option<Off> {
    env.map(Off::Env)
        .or((!enabled).then_some(Off::Setting))
        .or((!endpoint).then_some(Off::Unofficial))
        .or(unsigned.then_some(Off::Unsigned))
}

/// The running launcher's telemetry, once [`start`] made it.
pub fn get() -> Option<&'static Telemetry> {
    TELEMETRY.get()
}

static TELEMETRY: OnceLock<Telemetry> = OnceLock::new();

/// Starts the worker, when this build can send at all, with `telemetry` as config.toml sets it.
pub fn start(enabled: bool) -> &'static Telemetry {
    TELEMETRY.get_or_init(|| {
        let telemetry = Telemetry::new(ENDPOINT.map(str::to_owned), env_off(env_var), enabled);
        if telemetry.shared.can_send() {
            let worker = Worker {
                shared: telemetry.shared.clone(),
                storage: crate::extensions::access::shared_storage(),
                store: None,
                send: post,
            };
            let spawned = std::thread::Builder::new()
                .name("dupin-telemetry".into())
                .spawn(move || worker.run());
            if spawned.is_err() {
                crate::log::warn("telemetry is off: its thread couldn't start");
            }
        }
        telemetry
    })
}

pub struct Telemetry {
    shared: Arc<Shared>,
}

struct Shared {
    endpoint: Option<String>,
    env_off: Option<&'static str>,
    /// `telemetry` in config.toml.
    enabled: AtomicBool,
    unsigned: AtomicBool,
    /// The panel should tell the user before the first ping.
    notice_due: AtomicBool,
    /// This launch told the user, so it sends nothing.
    noticed_here: AtomicBool,
    /// Set to wake the worker before its next check.
    wake: Mutex<bool>,
    signal: Condvar,
}

impl Shared {
    fn can_send(&self) -> bool {
        self.endpoint.is_some() && self.env_off.is_none()
    }

    fn sending(&self) -> bool {
        decide(
            self.env_off,
            self.enabled.load(Ordering::SeqCst),
            self.endpoint.is_some(),
            self.unsigned.load(Ordering::SeqCst),
        )
        .is_none()
    }

    fn wake(&self) {
        *self.wake.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.signal.notify_all();
    }

    /// Waits `timeout`, or until woken.
    fn sleep(&self, timeout: Duration) {
        let woken = self.wake.lock().unwrap_or_else(PoisonError::into_inner);
        let (mut woken, _) = self
            .signal
            .wait_timeout_while(woken, timeout, |woken| !*woken)
            .unwrap_or_else(PoisonError::into_inner);
        *woken = false;
    }
}

impl Telemetry {
    fn new(endpoint: Option<String>, env_off: Option<&'static str>, enabled: bool) -> Telemetry {
        Telemetry {
            shared: Arc::new(Shared {
                endpoint,
                env_off,
                enabled: AtomicBool::new(enabled),
                unsigned: AtomicBool::new(false),
                notice_due: AtomicBool::new(false),
                noticed_here: AtomicBool::new(false),
                wake: Mutex::new(false),
                signal: Condvar::new(),
            }),
        }
    }

    /// Follows `telemetry` in config.toml. Off, nothing more is sent from now on and the worker
    /// deletes the stored ID.
    pub fn set_enabled(&self, enabled: bool) {
        if self.shared.enabled.swap(enabled, Ordering::SeqCst) != enabled {
            if !enabled {
                self.shared.notice_due.store(false, Ordering::SeqCst);
            }
            self.shared.wake();
        }
    }

    /// The first-run welcome showed what is sent.
    pub fn notice_shown(&self) {
        self.shared.notice_due.store(false, Ordering::SeqCst);
        if !self.shared.noticed_here.swap(true, Ordering::SeqCst) {
            self.shared.wake();
        }
    }

    /// The notice the panel should show as it opens, once, while the user hasn't been told.
    pub fn take_notice(&self) -> Option<&'static str> {
        if !self.shared.notice_due.swap(false, Ordering::SeqCst) || !self.shared.sending() {
            return None;
        }
        self.notice_shown();
        Some(NOTICE)
    }
}

struct Worker {
    shared: Arc<Shared>,
    storage: Option<Arc<Storage>>,
    store: Option<Store>,
    send: fn(&str, &str) -> Result<(), String>,
}

impl Worker {
    fn run(mut self) {
        if let Err(problem) = crate::update::developer_id_requirement(&dupin_mac::bundle::path()) {
            self.shared.unsigned.store(true, Ordering::SeqCst);
            crate::log::warn(&format!("telemetry is off: {problem}"));
            return;
        }
        let mut wait = FIRST_CHECK;
        loop {
            self.shared.sleep(wait);
            if let Err(problem) = self.tick(unix_seconds()) {
                crate::log::warn(&format!("telemetry: {problem}"));
            }
            wait = RECHECK;
        }
    }

    fn tick(&mut self, now: u64) -> Result<(), String> {
        if !self.shared.sending() {
            return if self.shared.enabled.load(Ordering::SeqCst) {
                Ok(())
            } else {
                self.forget()
            };
        }
        let mut state = self.load(true)?.unwrap_or_default();
        let noticed_here = self.shared.noticed_here.load(Ordering::SeqCst);
        match plan(&mut state, now, noticed_here) {
            Plan::Notice => self.shared.notice_due.store(true, Ordering::SeqCst),
            Plan::Nothing => {}
            Plan::Save => self.save(&state)?,
            Plan::Send => {
                let id = state.anonymous_id.get_or_insert_with(new_anonymous_id).clone();
                // Stored before sending, so a failed ping isn't retried that day.
                self.save(&state)?;
                let body = serde_json::to_string(&payload(id)).map_err(|e| e.to_string())?;
                // The setting may have changed while the state was saved.
                if let Some(endpoint) = self.shared.endpoint.as_deref()
                    && self.shared.sending()
                    && (self.send)(endpoint, &body).is_err()
                {
                    crate::log::warn("telemetry: the daily ping wasn't sent");
                }
            }
        }
        Ok(())
    }

    /// Deletes the anonymous ID and when it last sent, keeping that the user was told.
    fn forget(&mut self) -> Result<(), String> {
        let Some(mut state) = self.load(false)? else {
            return Ok(());
        };
        if state.anonymous_id.is_none() && state.last_sent_day.is_none() {
            return Ok(());
        }
        state.anonymous_id = None;
        state.last_sent_day = None;
        self.save(&state)
    }

    /// The open namespace. With nothing ever stored and `create` false, it opens nothing.
    fn namespace(&mut self, create: bool) -> Result<Option<&Store>, String> {
        if self.store.is_none() {
            let storage = self.storage.as_ref().ok_or("there is no data directory")?;
            if !create && !storage.exists(NAMESPACE) {
                return Ok(None);
            }
            self.store = Some(storage.open(NAMESPACE, LIMITS).map_err(|e| e.to_string())?);
        }
        Ok(self.store.as_ref())
    }

    /// The stored state; an unreadable one is an error, never a fresh start.
    fn load(&mut self, create: bool) -> Result<Option<State>, String> {
        let Some(store) = self.namespace(create)? else {
            return Ok(None);
        };
        match store.get(&store.key_id(RECORD)).map_err(|e| e.to_string())? {
            Some(record) => serde_json::from_value(record.data)
                .map(Some)
                .map_err(|_| "the stored state is unreadable".to_owned()),
            None => Ok(Some(State::default())),
        }
    }

    fn save(&mut self, state: &State) -> Result<(), String> {
        let data = serde_json::to_value(state).map_err(|e| e.to_string())?;
        let store = self
            .namespace(true)?
            .ok_or("the telemetry namespace couldn't be opened")?;
        store
            .upsert(
                RECORD,
                NewRecord {
                    data,
                    blobs: Vec::new(),
                },
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

fn payload(anonymous_id: String) -> Ping {
    let version = dupin_mac::bundle::info_string("CFBundleShortVersionString").unwrap_or_default();
    Ping {
        event: PingEvent::DupinDailyActive,
        anonymous_id,
        channel: channel(&version),
        version,
        os: format!("macos {}", macos_major()),
        arch: std::env::consts::ARCH.to_owned(),
    }
}

/// `scripts/package-app.nu` versions a nightly `X.Y.Z-nightly.N+SHA`.
fn channel(version: &str) -> Channel {
    if version.contains("-nightly") {
        Channel::Nightly
    } else {
        Channel::Stable
    }
}

fn macos_major() -> isize {
    objc2_foundation::NSProcessInfo::processInfo()
        .operatingSystemVersion()
        .majorVersion
}

fn new_anonymous_id() -> String {
    let mut bytes = [0u8; 16];
    // SAFETY: The buffer is valid for its length; arc4random_buf has no other preconditions.
    unsafe { libc::arc4random_buf(bytes.as_mut_ptr().cast(), bytes.len()) };
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// POSTs `body` to `url` once, waiting at most [`SEND_TIMEOUT`].
fn post(url: &str, body: &str) -> Result<(), String> {
    let mut child = crate::http::curl(
        SEND_TIMEOUT,
        &[
            "--request",
            "POST",
            "--header",
            "content-type: application/json",
            "--data-binary",
            "@-",
            "--output",
            "/dev/null",
        ],
    )
    .arg(url)
    .stdin(Stdio::piped())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .map_err(|e| e.to_string())?;
    let written = child
        .stdin
        .take()
        .ok_or_else(|| "curl's stdin is unavailable".to_owned())
        .and_then(|mut stdin| stdin.write_all(body.as_bytes()).map_err(|e| e.to_string()));
    let status = child.wait().map_err(|e| e.to_string())?;
    written?;
    status.success().then_some(()).ok_or_else(|| "curl failed".into())
}

/// What `dupin telemetry status` prints, from the running launcher.
pub fn status(enabled: bool) -> serde_json::Value {
    let off = off(enabled);
    serde_json::json!({
        "sending": off.is_none(),
        "setting": enabled,
        "off": off.map(Off::describe),
        "docs": DOCS_URL,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Read};
    use std::net::TcpListener;
    use std::sync::mpsc;

    use super::*;

    const DAY: u64 = 20_000 * SECONDS_PER_DAY;

    fn lookup(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        move |name| vars.get(name).cloned()
    }

    #[test]
    fn either_variable_turns_it_off_unless_empty_zero_or_false() {
        assert_eq!(env_off(lookup(&[])), None);
        assert_eq!(env_off(lookup(&[("DO_NOT_TRACK", "1")])), Some("DO_NOT_TRACK"));
        assert_eq!(
            env_off(lookup(&[
                ("DUPIN_TELEMETRY_DISABLED", "yes"),
                ("DO_NOT_TRACK", "1")
            ])),
            Some("DUPIN_TELEMETRY_DISABLED")
        );
        for value in ["", " ", "0", "false", "FALSE"] {
            assert_eq!(
                env_off(lookup(&[
                    ("DO_NOT_TRACK", value),
                    ("DUPIN_TELEMETRY_DISABLED", value)
                ])),
                None,
                "{value:?}"
            );
        }
    }

    #[test]
    fn every_opt_out_wins_over_a_build_that_could_send() {
        assert_eq!(decide(None, true, true, false), None);
        assert_eq!(
            decide(Some("DO_NOT_TRACK"), true, true, false),
            Some(Off::Env("DO_NOT_TRACK"))
        );
        assert_eq!(decide(None, false, true, false), Some(Off::Setting));
        assert_eq!(decide(None, true, false, false), Some(Off::Unofficial));
        assert_eq!(decide(None, true, true, true), Some(Off::Unsigned));
    }

    #[test]
    fn a_build_without_the_collector_s_address_never_sends() {
        // `cargo test` builds have no address; only `nix build .#dupin-release` compiles one in.
        assert_eq!(ENDPOINT, None);
        assert_eq!(
            decide(None, true, ENDPOINT.is_some(), false),
            Some(Off::Unofficial)
        );
    }

    #[test]
    fn the_launch_that_tells_the_user_never_sends() {
        let mut state = State::default();
        assert_eq!(plan(&mut state, DAY, false), Plan::Notice);
        assert_eq!(state, State::default());

        assert_eq!(plan(&mut state, DAY + 10, true), Plan::Save);
        assert_eq!(state.noticed_at, Some(DAY + 10));
        assert_eq!(plan(&mut state, DAY + 2 * SECONDS_PER_DAY, true), Plan::Nothing);

        assert_eq!(plan(&mut state, DAY + 20, false), Plan::Send);
        assert_eq!(plan(&mut state, DAY + 3_600, false), Plan::Nothing);
        assert_eq!(plan(&mut state, DAY + SECONDS_PER_DAY, false), Plan::Send);
        assert_eq!(state.last_sent_day, Some(20_001));
    }

    #[test]
    fn the_payload_carries_only_the_documented_fields() {
        let body = serde_json::to_value(payload("ab".repeat(16))).unwrap();
        let mut keys: Vec<_> = body.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["anonymousId", "arch", "channel", "event", "os", "version"]);
        assert_eq!(body["event"], "dupin_daily_active");
        assert!(body["os"].as_str().unwrap().starts_with("macos "));
    }

    #[test]
    fn nightly_versions_report_the_nightly_channel() {
        assert_eq!(channel("0.5.0-nightly.1234+abc1234"), Channel::Nightly);
        assert_eq!(channel("0.5.0"), Channel::Stable);
    }

    #[test]
    fn anonymous_ids_are_random_hex() {
        let (first, second) = (new_anonymous_id(), new_anonymous_id());
        assert_eq!(first.len(), 32);
        assert!(
            first
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        );
        assert_ne!(first, second);
    }

    /// A collector on the loopback interface, which answers each request with 204 and reports
    /// its body.
    fn collector() -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/ping", listener.local_addr().unwrap());
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line.trim().is_empty() {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                stream
                    .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                    .unwrap();
                if sender.send(String::from_utf8(body).unwrap()).is_err() {
                    return;
                }
            }
        });
        (url, receiver)
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        worker: Worker,
        telemetry: Telemetry,
        received: mpsc::Receiver<String>,
    }

    fn fixture(env_off: Option<&'static str>) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(
            dir.path().join("storage"),
            dupin_storage::FileKey::new(dir.path().join("key")),
        );
        let (url, received) = collector();
        let telemetry = Telemetry::new(Some(url), env_off, true);
        let worker = Worker {
            shared: telemetry.shared.clone(),
            storage: Some(Arc::new(storage)),
            store: None,
            send: post,
        };
        Fixture {
            _dir: dir,
            worker,
            telemetry,
            received,
        }
    }

    fn stored(worker: &mut Worker) -> State {
        worker.load(false).unwrap().unwrap_or_default()
    }

    const QUIET: Duration = Duration::from_millis(300);

    #[test]
    fn a_later_launch_sends_one_ping_a_day_to_the_collector() {
        let mut f = fixture(None);
        f.worker.tick(DAY).unwrap();
        assert_eq!(
            f.telemetry.take_notice(),
            Some(NOTICE),
            "the panel tells the user first"
        );
        f.worker.tick(DAY + 1).unwrap();
        assert!(
            f.received.recv_timeout(QUIET).is_err(),
            "the launch that told them sends nothing"
        );

        // The next launch.
        f.worker.shared.noticed_here.store(false, Ordering::SeqCst);
        f.worker.tick(DAY + 2).unwrap();
        let body: serde_json::Value =
            serde_json::from_str(&f.received.recv_timeout(SEND_TIMEOUT).unwrap()).unwrap();
        let id = stored(&mut f.worker).anonymous_id.unwrap();
        assert_eq!(body["anonymousId"], id.as_str());
        assert_eq!(body["event"], "dupin_daily_active");

        f.worker.tick(DAY + 3).unwrap();
        assert!(f.received.recv_timeout(QUIET).is_err(), "one a UTC day");
        f.worker.tick(DAY + SECONDS_PER_DAY).unwrap();
        let next: serde_json::Value =
            serde_json::from_str(&f.received.recv_timeout(SEND_TIMEOUT).unwrap()).unwrap();
        assert_eq!(next["anonymousId"], id.as_str(), "the ID persists");
    }

    #[test]
    fn turning_the_setting_off_stops_the_next_ping_and_deletes_the_id() {
        let mut f = fixture(None);
        f.telemetry.notice_shown();
        f.worker.tick(DAY).unwrap();
        f.worker.shared.noticed_here.store(false, Ordering::SeqCst);
        f.worker.tick(DAY + 1).unwrap();
        f.received.recv_timeout(SEND_TIMEOUT).unwrap();
        assert!(stored(&mut f.worker).anonymous_id.is_some());

        f.telemetry.set_enabled(false);
        f.worker.tick(DAY + SECONDS_PER_DAY).unwrap();
        assert!(f.received.recv_timeout(QUIET).is_err());
        let state = stored(&mut f.worker);
        assert_eq!(state.anonymous_id, None);
        assert_eq!(state.last_sent_day, None);
        assert!(
            state.noticed_at.is_some(),
            "turning it back on needs no second notice"
        );
        assert_eq!(f.telemetry.take_notice(), None);

        f.telemetry.set_enabled(true);
        f.worker.tick(DAY + SECONDS_PER_DAY + 1).unwrap();
        let body: serde_json::Value =
            serde_json::from_str(&f.received.recv_timeout(SEND_TIMEOUT).unwrap()).unwrap();
        assert_ne!(body["anonymousId"], serde_json::Value::Null);
    }

    #[test]
    fn the_environment_s_opt_out_sends_nothing_and_touches_no_storage() {
        let mut f = fixture(Some("DO_NOT_TRACK"));
        assert!(!f.telemetry.shared.can_send(), "no worker starts");
        f.telemetry.notice_shown();
        for day in 0..3 {
            f.worker.tick(DAY + day * SECONDS_PER_DAY).unwrap();
        }
        assert!(f.received.recv_timeout(QUIET).is_err());
        assert_eq!(f.telemetry.take_notice(), None);
        assert!(f.worker.load(false).unwrap().is_none(), "nothing was stored");
    }

    #[test]
    fn a_ping_is_never_sent_without_a_notice() {
        let mut f = fixture(None);
        for day in 0..3 {
            f.worker.tick(DAY + day * SECONDS_PER_DAY).unwrap();
        }
        assert!(f.received.recv_timeout(QUIET).is_err());
        assert_eq!(stored(&mut f.worker), State::default());
    }

    #[test]
    fn an_unreadable_state_sends_nothing() {
        let mut f = fixture(None);
        let store = f.worker.namespace(true).unwrap().unwrap();
        store
            .upsert(
                RECORD,
                NewRecord {
                    data: serde_json::json!({"noticedAt": "soon"}),
                    blobs: Vec::new(),
                },
            )
            .unwrap();
        assert!(f.worker.tick(DAY).is_err());
        assert!(f.received.recv_timeout(QUIET).is_err());
    }
}
