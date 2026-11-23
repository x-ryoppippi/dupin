use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
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
                            if let Some(snapshot) = read_cache(directory)
                                && shared.publish(snapshot)
                            {
                                notify();
                            }
                            online.then(|| refresh(directory, &shared, &fetch, &notify).unwrap_or(INTERVAL))
                        });
                    match delay {
                        Some(delay) => match changes.recv_timeout(delay) {
                            Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        },
                        None if changes.recv().is_err() => break,
                        None => {}
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
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(directory.join(ATTEMPT_FILE))?;
    if marker.try_lock().is_err() {
        return Ok(INTERVAL);
    }
    let metadata = marker.metadata()?;
    if !metadata.is_file() {
        return Ok(INTERVAL);
    }
    if metadata.len() != 0 {
        let age = metadata.modified()?.elapsed().unwrap_or_default();
        if age < INTERVAL {
            return Ok(INTERVAL - age);
        }
    }
    if !state.enabled() {
        return Ok(INTERVAL);
    }
    // Record attempts before contacting ECB so failures and restarts share the same limit.
    marker.write_all(b"1")?;
    marker.sync_data()?;
    if state.enabled()
        && let Some(xml) = fetch()
        && let Some(rates) = Rates::parse(&xml)
        && state
            .snapshot()
            .is_none_or(|previous| previous.rates.date <= rates.date)
    {
        let refreshed = SystemTime::now();
        let saved = write_cache(directory, &xml);
        if state.publish(Snapshot {
            rates: Arc::new(rates),
            refreshed,
        }) {
            notify();
        }
        saved?;
    } else if state.snapshot().is_some_and(|snapshot| snapshot.is_stale()) {
        notify();
    }
    Ok(INTERVAL)
}

fn write_cache(directory: &Path, xml: &str) -> io::Result<()> {
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(xml.as_bytes())?;
    temporary.persist(directory.join(XML_FILE))?;
    Ok(())
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
        refresh(path, &state, &fetch, &|| {}).unwrap();
        assert_eq!(attempts.get(), 1);
        let restarted = State {
            enabled: AtomicBool::new(true),
            ..State::default()
        };
        refresh(path, &restarted, &fetch, &|| {}).unwrap();
        assert_eq!(attempts.get(), 1);
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
        for bad in ["<html/>".into(), FIXTURE.replace("2026-11-20", "2026-11-19")] {
            let marker = fs::OpenOptions::new()
                .write(true)
                .open(path.join(ATTEMPT_FILE))
                .unwrap();
            marker.set_modified(SystemTime::now() - INTERVAL).unwrap();
            refresh(path, &state, &|| Some(bad.clone()), &|| {}).unwrap();
            assert_eq!(fs::read_to_string(path.join(XML_FILE)).unwrap(), FIXTURE);
        }
        assert_eq!(read_cache(path).unwrap().rates.date, "2026-11-20");
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
