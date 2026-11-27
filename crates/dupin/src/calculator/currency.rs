use std::collections::HashMap;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock, mpsc};
use std::time::{Duration, SystemTime};

const FEED: &str = "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml";
const XML_FILE: &str = "eurofxref-daily.xml";
const ATTEMPT_FILE: &str = "last-attempt";
const MAX_XML_BYTES: usize = 64 * 1024;
const INTERVAL: Duration = Duration::from_hours(12);
const FIRST_RETRY: Duration = Duration::from_mins(5);

#[derive(Debug, PartialEq)]
pub(super) struct Rates {
    pub date: String,
    pub values: HashMap<String, f64>,
}

impl Rates {
    pub fn parse(xml: &str) -> Option<Self> {
        const GESMES: &str = "http://www.gesmes.org/xml/2002-08-01";
        const ECB: &str = "http://www.ecb.int/vocabulary/2002-08-01/eurofxref";
        let document = roxmltree::Document::parse(xml).ok()?;
        let root = document.root_element();
        if !root.has_tag_name((GESMES, "Envelope")) {
            return None;
        }
        let cubes = root.children().find(|node| node.has_tag_name((ECB, "Cube")))?;
        let observation = cubes.children().find(|node| node.has_tag_name((ECB, "Cube")))?;
        let date = observation.attribute("time")?;
        let [year, month, day]: [&str; 3] = date.split('-').collect::<Vec<_>>().try_into().ok()?;
        if (year.len(), month.len(), day.len()) != (4, 2, 2)
            || !date.bytes().all(|byte| byte == b'-' || byte.is_ascii_digit())
        {
            return None;
        }
        let (year, month, day) = (
            year.parse::<u32>().ok()?,
            month.parse::<usize>().ok()?,
            day.parse::<u32>().ok()?,
        );
        let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let days = [31, 28 + u32::from(leap), 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        if year == 0 || !(1..=12).contains(&month) || !(1..=days[month - 1]).contains(&day) {
            return None;
        }
        let mut values = HashMap::from([("EUR".into(), 1.0)]);
        for node in observation.children().filter(roxmltree::Node::is_element) {
            if !node.has_tag_name((ECB, "Cube")) {
                return None;
            }
            let code = node.attribute("currency")?;
            let rate: f64 = node.attribute("rate")?.parse().ok()?;
            if code.len() != 3
                || !code.bytes().all(|byte| byte.is_ascii_uppercase())
                || !rate.is_finite()
                || rate <= 0.0
            {
                return None;
            }
            if values.insert(code.into(), rate).is_some() {
                return None;
            }
        }
        (values.len() > 1).then(|| Rates {
            date: date.into(),
            values,
        })
    }
}

#[derive(Clone)]
pub(super) struct Snapshot {
    pub rates: Arc<Rates>,
    pub refreshed: SystemTime,
}

impl Snapshot {
    pub fn is_stale(&self) -> bool {
        self.refreshed.elapsed().is_ok_and(|age| age >= INTERVAL)
    }
}

#[derive(Default)]
pub(super) struct State {
    enabled: AtomicBool,
    snapshot: RwLock<Option<Snapshot>>,
}

impl State {
    #[cfg(test)]
    pub(super) fn with_snapshot(snapshot: Option<Snapshot>) -> Self {
        Self {
            enabled: AtomicBool::new(true),
            snapshot: RwLock::new(snapshot),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    pub fn snapshot(&self) -> Option<Snapshot> {
        self.snapshot.read().unwrap().clone()
    }

    fn publish(&self, snapshot: Snapshot) -> bool {
        let mut current = self.snapshot.write().unwrap();
        if let Some(previous) = current.as_ref()
            && (previous.rates.date > snapshot.rates.date
                || (previous.refreshed == snapshot.refreshed && previous.rates == snapshot.rates))
        {
            return false;
        }
        *current = Some(snapshot);
        true
    }
}

pub(super) struct Worker {
    pub state: Arc<State>,
    changed: mpsc::Sender<()>,
}

impl Worker {
    pub fn start(notify: impl Fn() + Send + 'static) -> Self {
        let directory = std::env::var_os("XDG_CACHE_HOME")
            .filter(|path| !path.is_empty())
            .map(|path| PathBuf::from(path).join("dupin"))
            .or_else(|| std::env::var_os("HOME").map(|path| PathBuf::from(path).join("Library/Caches/Dupin")))
            .map(|path| path.join("calculator"));
        Self::spawn(directory, !rune::tooling::is_headless(), download, notify)
    }

    fn spawn(
        directory: Option<PathBuf>,
        online: bool,
        fetch: impl Fn() -> Option<String> + Send + 'static,
        notify: impl Fn() + Send + 'static,
    ) -> Self {
        let state = Arc::new(State::default());
        let (changed, changes) = mpsc::channel();
        let shared = state.clone();
        std::thread::Builder::new()
            .name("dupin-exchange-rates".into())
            .spawn(move || {
                loop {
                    let delay = directory
                        .as_deref()
                        .filter(|_| shared.enabled())
                        .and_then(|directory| {
                            if online {
                                Some(refresh(directory, &shared, &fetch, &notify).unwrap_or_else(|_| {
                                    publish_cache(directory, &shared, &notify);
                                    FIRST_RETRY
                                }))
                            } else {
                                publish_cache(directory, &shared, &notify);
                                None
                            }
                        });
                    if !wait_for_change(&changes, delay) {
                        break;
                    }
                }
            })
            .expect("spawn exchange-rate worker");
        Self { state, changed }
    }

    pub fn set_enabled(&self, enabled: bool) -> bool {
        let changed = self.state.enabled.swap(enabled, Ordering::AcqRel) != enabled;
        if changed {
            let _ = self.changed.send(());
        }
        changed
    }
}

fn wait_for_change(changes: &mpsc::Receiver<()>, delay: Option<Duration>) -> bool {
    let Some(delay) = delay else {
        return changes.recv().is_ok();
    };
    let deadline = SystemTime::now() + delay;
    loop {
        let wait = crate::wall_clock::wait_slice(deadline, SystemTime::now());
        if wait.is_zero() {
            return true;
        }
        match changes.recv_timeout(wait) {
            Ok(()) => return true,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return false,
        }
    }
}

fn publish_cache(directory: &Path, state: &State, notify: &impl Fn()) -> Option<SystemTime> {
    let snapshot = read_cache(directory)?;
    let refreshed = snapshot.refreshed;
    if state.publish(snapshot) {
        notify();
    }
    Some(refreshed)
}

fn read_cache(directory: &Path) -> Option<Snapshot> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(directory.join(XML_FILE))
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_XML_BYTES as u64 {
        return None;
    }
    let mut xml = String::new();
    file.take(MAX_XML_BYTES as u64 + 1)
        .read_to_string(&mut xml)
        .ok()?;
    let rates = (xml.len() <= MAX_XML_BYTES).then(|| Rates::parse(&xml))??;
    Some(Snapshot {
        rates: Arc::new(rates),
        refreshed: metadata.modified().ok()?,
    })
}

fn refresh(
    directory: &Path,
    state: &State,
    fetch: &impl Fn() -> Option<String>,
    notify: &impl Fn(),
) -> io::Result<Duration> {
    if !state.enabled() {
        return Ok(INTERVAL);
    }
    fs::create_dir_all(directory)?;
    let mut marker = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(directory.join(ATTEMPT_FILE))?;
    match marker.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            publish_cache(directory, state, notify);
            return Ok(Duration::from_secs(15));
        }
        Err(TryLockError::Error(error)) => return Err(error),
    }
    let result = refresh_locked(directory, &mut marker, state, fetch, notify);
    // A forked child can keep the lock alive after our descriptor closes.
    marker.unlock()?;
    result
}

fn refresh_locked(
    directory: &Path,
    marker: &mut File,
    state: &State,
    fetch: &impl Fn() -> Option<String>,
    notify: &impl Fn(),
) -> io::Result<Duration> {
    let metadata = marker.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::other("the exchange-rate marker is not a file"));
    }
    // Read under the lock so a successful refresh by another instance also resets our schedule.
    if let Some(refreshed) = publish_cache(directory, state, notify) {
        let remaining = INTERVAL.saturating_sub(refreshed.elapsed().unwrap_or_default());
        if !remaining.is_zero() {
            return Ok(remaining);
        }
    }
    let mut retry = FIRST_RETRY;
    if metadata.len() != 0 {
        let mut stored = Vec::new();
        Read::by_ref(marker).take(16).read_to_end(&mut stored)?;
        let previous = std::str::from_utf8(&stored)
            .ok()
            .and_then(|text| text.trim().parse::<u64>().ok())
            .map(Duration::from_secs)
            .filter(|delay| (FIRST_RETRY..=INTERVAL).contains(delay));
        // Older builds left a one-byte marker, so give it only the initial cooldown.
        let delay = previous.unwrap_or(FIRST_RETRY);
        let age = metadata.modified()?.elapsed().unwrap_or_default();
        if age < delay {
            return Ok(delay - age);
        }
        retry = previous.map_or(FIRST_RETRY, |delay| (delay * 2).min(INTERVAL));
    }
    if !state.enabled() {
        return Ok(INTERVAL);
    }
    // Record the retry before contacting the ECB so a restart cannot bypass the backoff.
    let recorded = format!("{}\n", retry.as_secs());
    marker.rewind()?;
    marker.write_all(recorded.as_bytes())?;
    marker.set_len(recorded.len() as u64)?;
    marker.sync_data()?;
    if state.enabled()
        && let Some(xml) = fetch()
        && let Some(rates) = Rates::parse(&xml)
        && state
            .snapshot()
            .is_none_or(|previous| previous.rates.date <= rates.date)
    {
        let saved = write_cache(directory, &xml);
        let refreshed = saved.as_ref().copied().unwrap_or_else(|_| SystemTime::now());
        if state.publish(Snapshot {
            rates: Arc::new(rates),
            refreshed,
        }) {
            notify();
        }
        saved?;
        marker.set_len(0)?;
        return Ok(INTERVAL);
    } else if state.snapshot().is_some_and(|snapshot| snapshot.is_stale()) {
        notify();
    }
    Ok(retry)
}

fn write_cache(directory: &Path, xml: &str) -> io::Result<SystemTime> {
    let destination = directory.join(XML_FILE);
    let temporary = destination.with_extension("xml.tmp");
    // The refresh lock gives this writer exclusive access, including to a previous crash's file.
    match fs::remove_file(&temporary) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(xml.as_bytes())?;
        let refreshed = file.metadata()?.modified()?;
        fs::rename(&temporary, destination)?;
        Ok(refreshed)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn download() -> Option<String> {
    let output = crate::http::curl(Duration::from_secs(10), &["--disable"])
        .args(["--proto", "=https", "--max-filesize"])
        .arg(MAX_XML_BYTES.to_string())
        .arg(FEED)
        .output()
        .ok()?;
    (output.status.success() && output.stdout.len() <= MAX_XML_BYTES)
        .then(|| String::from_utf8(output.stdout).ok())
        .flatten()
}

#[cfg(test)]
const FIXTURE: &str = include_str!("../../tests/fixtures/eurofxref-daily.xml");

#[cfg(test)]
pub(super) fn fixture() -> Snapshot {
    Snapshot {
        rates: Arc::new(Rates::parse(FIXTURE).unwrap()),
        refreshed: SystemTime::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_valid_ecb_rates() {
        let rates = Rates::parse(FIXTURE).unwrap();
        assert_eq!(rates.date, "2026-11-20");
        assert_eq!(rates.values["EUR"], 1.0);
        assert_eq!(rates.values["JPY"], 164.45);
        for bad in [
            FIXTURE.replace("2026-11-20", "2026-02-29"),
            FIXTURE.replace("164.45", "NaN"),
            FIXTURE.replace("164.45", "0"),
            FIXTURE.replace("JPY", "USD"),
            FIXTURE.trim_end_matches("</gesmes:Envelope>\n").into(),
        ] {
            assert!(Rates::parse(&bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn refresh_requires_consent_and_throttles_failures_across_instances() {
        use std::cell::Cell;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        let attempts = Cell::new(0);
        let fetch = || {
            attempts.set(attempts.get() + 1);
            None
        };
        let state = State::default();
        refresh(path, &state, &fetch, &|| {}).unwrap();
        assert_eq!(attempts.get(), 0);
        assert!(!path.join(ATTEMPT_FILE).exists());

        state.enabled.store(true, Ordering::Release);
        assert_eq!(refresh(path, &state, &fetch, &|| {}).unwrap(), FIRST_RETRY);
        assert_eq!(attempts.get(), 1);
        let restarted = State {
            enabled: AtomicBool::new(true),
            ..State::default()
        };
        let wait = refresh(path, &restarted, &fetch, &|| {}).unwrap();
        assert!(wait > Duration::ZERO && wait <= FIRST_RETRY);
        assert_eq!(attempts.get(), 1);
        let marker = fs::OpenOptions::new()
            .write(true)
            .open(path.join(ATTEMPT_FILE))
            .unwrap();
        for retry in [FIRST_RETRY * 2, FIRST_RETRY * 4, INTERVAL] {
            if retry == INTERVAL {
                fs::write(path.join(ATTEMPT_FILE), format!("{}\n", INTERVAL.as_secs())).unwrap();
            }
            marker.set_modified(SystemTime::now() - INTERVAL).unwrap();
            assert_eq!(refresh(path, &restarted, &fetch, &|| {}).unwrap(), retry);
        }
        assert_eq!(attempts.get(), 4);
        marker.set_modified(SystemTime::now() - INTERVAL).unwrap();
        assert_eq!(
            refresh(path, &restarted, &|| Some(FIXTURE.into()), &|| {}).unwrap(),
            INTERVAL
        );
        assert_eq!(marker.metadata().unwrap().len(), 0);
        let remaining = refresh(path, &state, &|| panic!("fresh cache"), &|| {}).unwrap();
        assert!(remaining > INTERVAL - FIRST_RETRY && remaining <= INTERVAL);
        assert_eq!(state.snapshot().unwrap().rates.date, "2026-11-20");
    }

    #[test]
    fn refresh_preserves_the_last_good_xml_after_bad_responses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        let state = State {
            enabled: AtomicBool::new(true),
            ..State::default()
        };
        refresh(path, &state, &|| Some(FIXTURE.into()), &|| {}).unwrap();
        assert_eq!(fs::read_to_string(path.join(XML_FILE)).unwrap(), FIXTURE);
        fs::OpenOptions::new()
            .write(true)
            .open(path.join(XML_FILE))
            .unwrap()
            .set_modified(SystemTime::now() - INTERVAL)
            .unwrap();
        for bad in ["<html/>".into(), FIXTURE.replace("2026-11-20", "2026-11-19")] {
            let marker = fs::OpenOptions::new()
                .write(true)
                .open(path.join(ATTEMPT_FILE))
                .unwrap();
            marker.set_modified(SystemTime::now() - INTERVAL).unwrap();
            let attempts = std::cell::Cell::new(0);
            refresh(
                path,
                &state,
                &|| {
                    attempts.set(attempts.get() + 1);
                    Some(bad.clone())
                },
                &|| {},
            )
            .unwrap();
            assert_eq!(attempts.get(), 1);
            assert_eq!(fs::read_to_string(path.join(XML_FILE)).unwrap(), FIXTURE);
        }
        assert_eq!(read_cache(path).unwrap().rates.date, "2026-11-20");
    }

    #[test]
    fn cache_writer_replaces_an_abandoned_temporary_symlink_without_following_it() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("other");
        fs::write(&other, "original").unwrap();
        let temporary = dir.path().join(XML_FILE).with_extension("xml.tmp");
        std::os::unix::fs::symlink(&other, &temporary).unwrap();
        write_cache(dir.path(), FIXTURE).unwrap();
        assert!(!temporary.exists());
        assert_eq!(fs::read_to_string(other).unwrap(), "original");
        assert_eq!(fs::read_to_string(dir.path().join(XML_FILE)).unwrap(), FIXTURE);
    }

    #[test]
    fn headless_worker_loads_cache_without_fetching() {
        let dir = tempfile::tempdir().unwrap();
        write_cache(dir.path(), FIXTURE).unwrap();
        let (ready, updates) = mpsc::channel();
        let worker = Worker::spawn(
            Some(dir.path().to_owned()),
            false,
            || panic!("unexpected fetch"),
            move || ready.send(()).unwrap(),
        );
        assert!(worker.state.snapshot().is_none());
        worker.set_enabled(true);
        updates.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(worker.state.snapshot().unwrap().rates.date, "2026-11-20");
        assert!(!dir.path().join(ATTEMPT_FILE).exists());
    }
}
