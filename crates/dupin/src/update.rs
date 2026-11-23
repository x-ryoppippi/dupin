//! Self-update for the signed release builds.
//!
//! The release packaging writes the channel's feed URL into Info.plist (`DupinUpdateFeed`);
//! a build without one (Nix, `cargo run`) never updates, since Nix owns its store paths. The
//! feed is a small JSON file beside the release's zip. A newer build is unpacked beside the
//! installed bundle, in `.Dupin-update-<build>`, and kept only when it satisfies the running
//! bundle's own designated requirement (same bundle identifier, same Developer ID team) and
//! Gatekeeper accepts it. The installed bundle stays untouched until Dupin restarts into the
//! update or quits: then one `renamex_np(RENAME_SWAP)` exchanges the two, so the install path
//! is never empty, and the replaced bundle waits in the staging folder for the next launch to
//! remove it. A copy that can't replace itself (a read-only folder, App Translocation) only
//! says where to download the update. `DUPIN_DISABLE_UPDATES=1` turns the updater off.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rune::AppHandle;
use serde::Deserialize;

use crate::prefs::Prefs;
use crate::ui::Ui;

/// Long enough after launch not to compete with the first app scan and the panel's warm-up.
const FIRST_CHECK: Duration = Duration::from_secs(60);
const INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// The name the release zip gives the bundle, whatever the user renamed the installed copy to.
const ARCHIVED_BUNDLE: &str = "Dupin.app";
/// What a staging folder beside the installed bundle is named with. Only folders with this
/// prefix are ever removed.
const PREFIX: &str = ".Dupin-update-";

/// What a channel's `update.json` says about its newest build.
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct Feed {
    pub version: String,
    /// `CFBundleVersion`: the commit count of the build, which only grows along `main`.
    pub build: u64,
    pub url: String,
    pub sha256: String,
}

impl Feed {
    fn parse(json: &str) -> Result<Feed, String> {
        serde_json::from_str(json).map_err(|e| format!("the update feed is invalid: {e}"))
    }
}

/// How this copy takes an update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Method {
    /// Stage it beside the bundle and swap it in.
    Install,
    /// Point at the download: the bundle can't be replaced in place.
    DownloadOnly,
}

impl Method {
    /// Replacing the bundle renames it within its folder, so both must be writable, and a
    /// Gatekeeper App Translocation copy is a read-only mount that disappears with the process.
    fn of(bundle: &Path, writable: impl Fn(&Path) -> bool) -> Method {
        let translocated = bundle.components().any(|c| c.as_os_str() == "AppTranslocation");
        if !translocated && writable(bundle) && bundle.parent().is_some_and(&writable) {
            Method::Install
        } else {
            Method::DownloadOnly
        }
    }
}

fn writable(path: &Path) -> bool {
    CString::new(path.as_os_str().as_bytes())
        // SAFETY: The string is NUL-terminated and outlives the call; access has no side effects.
        .is_ok_and(|path| unsafe { libc::access(path.as_ptr(), libc::W_OK) } == 0)
}

fn off_by_env() -> bool {
    std::env::var_os("DUPIN_DISABLE_UPDATES").is_some_and(|v| v == "1")
}

/// The running build, when it can update itself.
struct Installed {
    bundle: PathBuf,
    feed: String,
    build: u64,
    /// The bundle's designated requirement, which an update must satisfy.
    requirement: String,
    method: Method,
}

impl Installed {
    fn current() -> Result<Option<Installed>, String> {
        let Some(feed) = dupin_mac::bundle::info_string("DupinUpdateFeed") else {
            return Ok(None);
        };
        let bundle = dupin_mac::bundle::path();
        let build = dupin_mac::bundle::info_string("CFBundleVersion")
            .and_then(|v| v.parse().ok())
            .ok_or("CFBundleVersion is not a build number")?;
        let requirement = designated_requirement(&bundle)?;
        // An ad-hoc signature's requirement is its own hash, which no other build satisfies.
        if !requirement.contains("anchor apple generic") {
            return Err(format!("{} is not signed with a Developer ID", bundle.display()));
        }
        Ok(Some(Installed {
            method: Method::of(&bundle, writable),
            bundle,
            feed,
            build,
            requirement,
        }))
    }
}

/// What a check found newer than the running build.
enum Found {
    Staged(Staged),
    Download { version: String, url: String },
}

/// A verified update unpacked beside the bundle it replaces.
#[derive(Debug)]
struct Staged {
    update: PathBuf,
    installed: PathBuf,
    version: String,
}

/// Where the updater is, as Settings shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// `DUPIN_DISABLE_UPDATES`, or a build that isn't signed with a Developer ID.
    Off,
    /// No check has run yet.
    Idle,
    Checking,
    Downloading,
    Current,
    Failed(String),
    /// Staged; installs on restart or quit.
    Ready(String),
    /// A newer build this copy can't install itself.
    Download {
        url: String,
    },
}

impl Status {
    pub fn text(&self, auto_update: bool, download_only: bool) -> String {
        match self {
            Status::Off => "Off".into(),
            Status::Checking => "Checking…".into(),
            Status::Downloading => "Downloading…".into(),
            Status::Failed(e) => format!("Couldn't update: {}", e.lines().next().unwrap_or_default()),
            Status::Ready(version) => format!("Ready: {version}"),
            Status::Download { .. } => "Download only".into(),
            Status::Idle | Status::Current if download_only => "Download only".into(),
            Status::Idle if !auto_update => "Off".into(),
            Status::Idle => "Not checked yet".into(),
            Status::Current => "Up to date".into(),
        }
    }

    /// Whether "Check for Updates" can start a check now.
    pub fn can_check(&self) -> bool {
        matches!(
            self,
            Status::Idle | Status::Current | Status::Failed(_) | Status::Download { .. }
        )
    }
}

/// The updater's state, shared by its worker, the panel and Settings.
#[derive(Clone)]
pub struct Updater {
    shared: Arc<Shared>,
}

struct Shared {
    /// `auto_update` in config.toml; the next scheduled check reads it.
    enabled: AtomicBool,
    /// This build has an update feed, so Settings shows its updates.
    available: bool,
    download_only: bool,
    version: String,
    inner: Mutex<Inner>,
    /// Set by "Check for Updates"; the worker waits on it between checks.
    wake: Mutex<bool>,
    signal: Condvar,
    prefs: AppHandle<Prefs>,
}

struct Inner {
    status: Status,
    staged: Option<Staged>,
    /// The bundles were swapped. A second swap would exchange them back, so nothing stages or
    /// swaps again in this process.
    applied: bool,
}

impl Updater {
    /// Start checking the channel's feed, when this build has one.
    pub fn start(enabled: bool, panel: AppHandle<Ui>, prefs: AppHandle<Prefs>) -> Updater {
        let installed = if off_by_env() {
            Ok(None)
        } else {
            Installed::current().inspect_err(|e| crate::log::warn(&format!("updates are off: {e}")))
        };
        let installed = installed.ok().flatten();
        let download_only = installed
            .as_ref()
            .is_some_and(|i| i.method == Method::DownloadOnly);
        let updater = Updater {
            shared: Arc::new(Shared {
                enabled: AtomicBool::new(enabled),
                available: has_feed(),
                download_only,
                version: dupin_mac::bundle::info_string("CFBundleShortVersionString").unwrap_or_default(),
                inner: Mutex::new(Inner {
                    status: if installed.is_some() {
                        Status::Idle
                    } else {
                        Status::Off
                    },
                    staged: None,
                    applied: false,
                }),
                wake: Mutex::new(false),
                signal: Condvar::new(),
                prefs,
            }),
        };
        let Some(installed) = installed else {
            return updater;
        };
        match installed.method {
            Method::Install => remove_beside(&installed.bundle),
            Method::DownloadOnly => crate::log::warn(&format!(
                "updates are download only: {} can't be replaced in place",
                installed.bundle.display()
            )),
        }
        let shared = updater.shared.clone();
        std::thread::Builder::new()
            .name("dupin-update".into())
            .spawn(move || run(&shared, &installed, &panel))
            .expect("spawn updater");
        updater
    }

    pub fn available(&self) -> bool {
        self.shared.available
    }

    /// `CFBundleShortVersionString` of the running build.
    pub fn version(&self) -> &str {
        &self.shared.version
    }

    pub fn status(&self) -> Status {
        self.shared.lock().status.clone()
    }

    pub fn download_only(&self) -> bool {
        self.shared.download_only
    }

    /// Turning updates on checks at once rather than at the next interval.
    pub fn set_enabled(&self, enabled: bool) {
        let was = self.shared.enabled.swap(enabled, Ordering::Relaxed);
        if enabled && !was {
            self.check_now();
        }
    }

    pub fn check_now(&self) {
        if !self.status().can_check() {
            return;
        }
        *self.shared.wake.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.shared.signal.notify_all();
    }

    /// Open the download a copy that can't install the update itself needs.
    pub fn open_download(&self) {
        let Status::Download { url } = self.status() else {
            return;
        };
        // The feed is unsigned: open a web page, never a file or another app's URL scheme.
        if !url.starts_with("https://") {
            return crate::log::warn(&format!("not opening the update download {url}"));
        }
        if let Err(e) = Command::new("/usr/bin/open").arg(&url).spawn() {
            crate::log::warn(&format!("couldn't open {url}: {e}"));
        }
    }

    /// Swap in the staged update. `Ok(false)` means no update was staged.
    pub fn install_staged(&self) -> Result<bool, String> {
        let Some(staged) = self.take_staged() else {
            return Ok(false);
        };
        match swap(&staged.update, &staged.installed) {
            Ok(()) => Ok(true),
            Err(e) => {
                let e = format!("couldn't install Dupin {}: {e}", staged.version);
                self.shared.lock().applied = false;
                self.shared.set_status(Status::Failed(e.clone()));
                Err(e)
            }
        }
    }

    /// Quitting with an update staged installs it for the next launch.
    pub fn install_on_quit(&self) {
        let Some(staged) = self.take_staged() else { return };
        if let Err(e) = swap(&staged.update, &staged.installed) {
            crate::log::warn(&format!("couldn't install Dupin {} at quit: {e}", staged.version));
        }
    }

    /// The staged update, which no later call sees: whoever takes it swaps.
    fn take_staged(&self) -> Option<Staged> {
        let mut inner = self.shared.lock();
        let staged = inner.staged.take()?;
        inner.applied = true;
        Some(staged)
    }
}

/// Whether this build has a channel to update from at all.
pub fn has_feed() -> bool {
    dupin_mac::bundle::info_string("DupinUpdateFeed").is_some()
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_status(&self, status: Status) {
        self.lock().status = status;
        self.prefs.update(|_, _| {});
    }

    /// Wait `delay` or until "Check for Updates"; true when asked.
    fn wait(&self, delay: Duration) -> bool {
        let deadline = Instant::now() + delay;
        let mut asked = self.wake.lock().unwrap_or_else(|e| e.into_inner());
        while !*asked {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            asked = self
                .signal
                .wait_timeout(asked, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        std::mem::take(&mut *asked)
    }
}

fn run(shared: &Shared, installed: &Installed, panel: &AppHandle<Ui>) {
    let mut asked = shared.wait(FIRST_CHECK);
    let mut announced = None;
    loop {
        let idle = {
            let inner = shared.lock();
            if inner.applied {
                return;
            }
            inner.staged.is_none()
        };
        if idle && (asked || shared.enabled.load(Ordering::Relaxed)) {
            shared.set_status(Status::Checking);
            match check(shared, installed) {
                Ok(Some(Found::Staged(staged))) => adopt(shared, staged, panel),
                Ok(Some(Found::Download { version, url })) => {
                    if announced.as_ref() != Some(&version) {
                        crate::log::warn(&format!("Dupin {version} is available: {url}"));
                        announced = Some(version);
                    }
                    shared.set_status(Status::Download { url });
                }
                Ok(None) => shared.set_status(Status::Current),
                Err(e) => {
                    crate::log::warn(&format!("couldn't update: {e}"));
                    shared.set_status(Status::Failed(e));
                }
            }
        }
        asked = shared.wait(INTERVAL);
    }
}

/// Keep `staged` for the restart, unless a swap already happened: the folder it is in then
/// may hold nothing else this process needs.
fn adopt(shared: &Shared, staged: Staged, panel: &AppHandle<Ui>) {
    let mut inner = shared.lock();
    if inner.applied {
        drop(inner);
        if let Some(folder) = staged.update.parent() {
            let _ = std::fs::remove_dir_all(folder);
        }
        return;
    }
    crate::log::warn(&format!("Dupin {} is ready to install", staged.version));
    inner.status = Status::Ready(staged.version.clone());
    inner.staged = Some(staged);
    drop(inner);
    shared.prefs.update(|_, _| {});
    panel.update(|s, tx| s.updated(tx));
}

/// Stage the feed's build if it is newer, or say where to get it when this copy can't
/// install it.
fn check(shared: &Shared, installed: &Installed) -> Result<Option<Found>, String> {
    let feed = Feed::parse(&output(curl(Duration::from_secs(60)).arg(&installed.feed))?)?;
    if feed.build <= installed.build {
        return Ok(None);
    }
    Ok(Some(match installed.method {
        Method::Install => {
            shared.set_status(Status::Downloading);
            Found::Staged(stage(installed, &feed)?)
        }
        Method::DownloadOnly => Found::Download {
            version: feed.version,
            url: feed.url,
        },
    }))
}

/// Download, unpack and verify the feed's build in its own folder beside the installed
/// bundle, on the same volume so the swap is one rename.
fn stage(installed: &Installed, feed: &Feed) -> Result<Staged, String> {
    let parent = installed
        .bundle
        .parent()
        .ok_or("the app bundle has no parent folder")?;
    let folder = parent.join(format!("{PREFIX}{}", feed.build));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir(&folder).map_err(|e| format!("couldn't create {}: {e}", folder.display()))?;
    match unpack_and_verify(installed, feed, &folder) {
        Ok(update) => Ok(Staged {
            update,
            installed: installed.bundle.clone(),
            version: feed.version.clone(),
        }),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&folder);
            Err(e)
        }
    }
}

fn unpack_and_verify(installed: &Installed, feed: &Feed, folder: &Path) -> Result<PathBuf, String> {
    let zip = folder.join("update.zip");
    output(
        curl(Duration::from_secs(15 * 60))
            .arg("--output")
            .arg(&zip)
            .arg(&feed.url),
    )?;

    let sum = output(Command::new("/usr/bin/shasum").args(["-a", "256"]).arg(&zip))?;
    if sum.split_whitespace().next() != Some(feed.sha256.as_str()) {
        return Err("the download doesn't match the feed's checksum".into());
    }

    output(
        Command::new("/usr/bin/ditto")
            .args(["-x", "-k"])
            .arg(&zip)
            .arg(folder),
    )?;
    let _ = std::fs::remove_file(&zip);
    let new = folder.join(ARCHIVED_BUNDLE);

    output(
        Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(format!("-R={}", installed.requirement))
            .arg(&new),
    )
    .map_err(|e| format!("the update isn't signed as this app is: {e}"))?;
    output(
        Command::new("/usr/sbin/spctl")
            .args(["--assess", "--type", "execute"])
            .arg(&new),
    )
    .map_err(|e| format!("Gatekeeper rejects the update: {e}"))?;

    // The feed is unsigned; the bundle's own Info.plist is covered by its signature, so it,
    // not the feed, decides that the update is newer and on this channel.
    let build = info_value(&new, "CFBundleVersion")?
        .parse::<u64>()
        .map_err(|_| "the update's CFBundleVersion is not a build number")?;
    if build <= installed.build {
        return Err(format!(
            "the update is build {build}, not newer than {}",
            installed.build
        ));
    }
    if info_value(&new, "DupinUpdateFeed")? != installed.feed {
        return Err("the update belongs to another channel".into());
    }
    Ok(new)
}

/// Exchange the two bundles in one step: afterwards `installed` holds the update and
/// `update` the bundle it replaced.
fn swap(update: &Path, installed: &Path) -> std::io::Result<()> {
    let from = CString::new(update.as_os_str().as_bytes())?;
    let to = CString::new(installed.as_os_str().as_bytes())?;
    // SAFETY: Both strings are NUL-terminated and outlive the call.
    if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_SWAP) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Remove the staging folders beside `bundle`: the bundle an update replaced, or an update
/// never installed. A link with the prefix stays, and so does the bundle itself.
fn remove_beside(bundle: &Path) {
    let Some(entries) = bundle.parent().and_then(|p| std::fs::read_dir(p).ok()) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ours = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(PREFIX));
        if ours
            && entry.file_type().is_ok_and(|t| t.is_dir())
            && path != bundle
            && let Err(e) = std::fs::remove_dir_all(&path)
        {
            crate::log::warn(&format!("couldn't remove {}: {e}", path.display()));
        }
    }
}

/// curl that fails on HTTP errors and gives up on a stalled server.
fn curl(max: Duration) -> Command {
    crate::http::curl(max, &["--location"])
}

fn info_value(bundle: &Path, key: &str) -> Result<String, String> {
    let value = output(
        Command::new("/usr/bin/plutil")
            .args(["-extract", key, "raw", "-o", "-"])
            .arg(bundle.join("Contents/Info.plist")),
    )?;
    Ok(value.trim().to_owned())
}

fn designated_requirement(bundle: &Path) -> Result<String, String> {
    let out = output(
        Command::new("/usr/bin/codesign")
            .args(["--display", "--requirements", "-"])
            .arg(bundle),
    )?;
    out.lines()
        .find_map(|line| line.trim_start_matches("# ").strip_prefix("designated => "))
        .map(str::to_owned)
        .ok_or_else(|| format!("{} has no designated requirement", bundle.display()))
}

/// Run to completion; stdout on success, stderr as the error otherwise.
fn output(command: &mut Command) -> Result<String, String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let out = command
        .output()
        .map_err(|e| format!("couldn't run {program}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_feed() {
        let feed = Feed::parse(
            r#"{"version":"0.2.0","build":42,"url":"https://example.com/Dupin.zip","sha256":"ab"}"#,
        )
        .unwrap();
        assert_eq!(
            feed,
            Feed {
                version: "0.2.0".into(),
                build: 42,
                url: "https://example.com/Dupin.zip".into(),
                sha256: "ab".into(),
            }
        );
    }

    #[test]
    fn rejects_a_feed_without_a_checksum() {
        assert!(Feed::parse(r#"{"version":"0.2.0","build":42,"url":"u"}"#).is_err());
    }

    #[test]
    fn installs_only_where_the_bundle_can_be_replaced() {
        let everywhere = |_: &Path| true;
        let app = Path::new("/Applications/Dupin.app");
        assert_eq!(Method::of(app, everywhere), Method::Install);
        assert_eq!(
            Method::of(app, |p: &Path| p != Path::new("/Applications")),
            Method::DownloadOnly
        );
        assert_eq!(Method::of(app, |p: &Path| p != app), Method::DownloadOnly);
        let translocated = Path::new("/private/var/folders/x/AppTranslocation/1234/d/Dupin.app");
        assert_eq!(Method::of(translocated, everywhere), Method::DownloadOnly);
    }

    #[test]
    fn says_where_the_updater_is() {
        let text = |status: Status| status.text(true, false);
        assert_eq!(text(Status::Off), "Off");
        assert_eq!(text(Status::Idle), "Not checked yet");
        assert_eq!(text(Status::Checking), "Checking…");
        assert_eq!(text(Status::Downloading), "Downloading…");
        assert_eq!(text(Status::Current), "Up to date");
        assert_eq!(text(Status::Ready("0.3.0".into())), "Ready: 0.3.0");
        assert_eq!(
            text(Status::Failed("/usr/bin/curl failed: timed out\nmore".into())),
            "Couldn't update: /usr/bin/curl failed: timed out"
        );
        assert_eq!(Status::Idle.text(false, false), "Off");
        assert_eq!(Status::Current.text(false, false), "Up to date");
        assert_eq!(Status::Current.text(true, true), "Download only");
        assert_eq!(
            text(Status::Download {
                url: "https://example.com/Dupin.zip".into()
            }),
            "Download only"
        );
    }

    #[test]
    fn checks_only_when_nothing_is_under_way() {
        assert!(Status::Current.can_check());
        assert!(Status::Failed("x".into()).can_check());
        assert!(!Status::Off.can_check());
        assert!(!Status::Checking.can_check());
        assert!(!Status::Downloading.can_check());
        assert!(!Status::Ready("0.3.0".into()).can_check());
    }

    #[test]
    fn a_temporary_folder_is_writable_and_the_system_is_not() {
        let root = temporary("writable");
        assert!(writable(&root));
        assert!(!writable(Path::new("/System/Applications")));
        let _ = std::fs::remove_dir_all(root);
    }

    fn temporary(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("dupin-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn bundle(path: &Path, marker: &str) {
        std::fs::create_dir_all(path.join("Contents")).unwrap();
        std::fs::write(path.join("Contents/marker"), marker).unwrap();
    }

    fn marker(path: &Path) -> String {
        std::fs::read_to_string(path.join("Contents/marker")).unwrap()
    }

    fn staged_updater(staged: Staged) -> Updater {
        let prefs: AppHandle<Prefs> = rune::App::new(rune::AppSpec::new(
            "test",
            (1.0, 1.0),
            || unreachable!(),
            |_, _| rune::el(),
        ))
        .handle();
        Updater {
            shared: Arc::new(Shared {
                enabled: AtomicBool::new(false),
                available: false,
                download_only: false,
                version: "test".into(),
                inner: Mutex::new(Inner {
                    status: Status::Ready(staged.version.clone()),
                    staged: Some(staged),
                    applied: false,
                }),
                wake: Mutex::new(false),
                signal: Condvar::new(),
                prefs,
            }),
        }
    }

    #[test]
    fn installing_a_staged_update_only_swaps_once_and_returns_to_the_caller() {
        let root = temporary("install-staged");
        let installed = root.join("Dupin.app");
        let update = root.join("update/Dupin.app");
        bundle(&installed, "old");
        bundle(&update, "new");
        let updater = staged_updater(Staged {
            update: update.clone(),
            installed: installed.clone(),
            version: "next".into(),
        });

        assert_eq!(updater.install_staged(), Ok(true));
        assert!(updater.shared.lock().applied);
        assert_eq!(marker(&installed), "new");
        assert_eq!(marker(&update), "old");
        assert_eq!(updater.install_staged(), Ok(false));
        assert_eq!(marker(&installed), "new");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_failed_install_reports_failure_without_applying_or_relaunching() {
        let root = temporary("install-failed");
        let installed = root.join("Dupin.app");
        bundle(&installed, "old");
        let updater = staged_updater(Staged {
            update: root.join("missing.app"),
            installed: installed.clone(),
            version: "next".into(),
        });

        assert!(updater.install_staged().is_err());
        assert!(!updater.shared.lock().applied);
        assert!(matches!(updater.status(), Status::Failed(_)));
        assert_eq!(marker(&installed), "old");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_swap_exchanges_the_bundles_and_launch_removes_the_old_one() {
        let root = temporary("swap");
        let installed = root.join("Dupin Nightly.app");
        bundle(&installed, "old");
        let folder = root.join(format!("{PREFIX}42"));
        let update = folder.join(ARCHIVED_BUNDLE);
        bundle(&update, "new");

        swap(&update, &installed).unwrap();
        assert_eq!(marker(&installed), "new");
        assert_eq!(marker(&update), "old");

        remove_beside(&installed);
        assert!(!folder.exists());
        assert_eq!(marker(&installed), "new");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn launch_cleanup_leaves_what_isnt_a_staging_folder() {
        let root = temporary("cleanup");
        let installed = root.join("Dupin.app");
        bundle(&installed, "running");
        bundle(&root.join("Other.app"), "other");
        std::fs::write(root.join(format!("{PREFIX}notes")), "a file").unwrap();
        let precious = root.join("precious");
        bundle(&precious, "precious");
        std::os::unix::fs::symlink(&precious, root.join(format!("{PREFIX}link"))).unwrap();
        let unused = root.join(format!("{PREFIX}7"));
        bundle(&unused.join(ARCHIVED_BUNDLE), "unused");

        remove_beside(&installed);
        assert!(!unused.exists());
        assert_eq!(marker(&installed), "running");
        assert!(root.join("Other.app").exists());
        assert!(root.join(format!("{PREFIX}notes")).exists());
        assert!(root.join(format!("{PREFIX}link")).exists());
        assert_eq!(marker(&precious), "precious");
        let _ = std::fs::remove_dir_all(root);
    }
}
