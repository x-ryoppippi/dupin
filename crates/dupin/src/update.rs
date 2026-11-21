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
//! remove it.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use rune::AppHandle;
use serde::Deserialize;

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

/// The running build, when it can update itself.
struct Installed {
    bundle: PathBuf,
    feed: String,
    build: u64,
    /// The bundle's designated requirement, which an update must satisfy.
    requirement: String,
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
            bundle,
            feed,
            build,
            requirement,
        }))
    }
}

/// A verified update unpacked beside the bundle it replaces.
#[derive(Debug)]
struct Staged {
    update: PathBuf,
    installed: PathBuf,
    version: String,
}

/// The updater's state, shared by its worker and the UI thread.
#[derive(Clone)]
pub struct Updater {
    shared: Arc<Shared>,
}

struct Shared {
    /// `auto_update` in config.toml; the next check reads it.
    enabled: AtomicBool,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    staged: Option<Staged>,
    /// The bundles were swapped. A second swap would exchange them back, so nothing stages or
    /// swaps again in this process.
    applied: bool,
}

impl Updater {
    /// Start checking the channel's feed, when this build has one.
    pub fn start(enabled: bool, panel: AppHandle<Ui>) -> Updater {
        let updater = Updater {
            shared: Arc::new(Shared {
                enabled: AtomicBool::new(enabled),
                inner: Mutex::new(Inner::default()),
            }),
        };
        let installed = match Installed::current() {
            Ok(Some(installed)) => installed,
            Ok(None) => return updater,
            Err(e) => {
                crate::log::warn(&format!("updates are off: {e}"));
                return updater;
            }
        };
        remove_beside(&installed.bundle);
        let shared = updater.shared.clone();
        std::thread::Builder::new()
            .name("dupin-update".into())
            .spawn(move || run(&shared, &installed, &panel))
            .expect("spawn updater");
        updater
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.shared.enabled.store(enabled, Ordering::Relaxed);
    }

    /// Swap in the staged update and start it once this process has exited, and exit.
    /// Returns when nothing is staged or the swap failed.
    pub fn restart(&self) {
        let Some(staged) = self.take_staged() else { return };
        match swap(&staged.update, &staged.installed) {
            Ok(()) => relaunch(&staged.installed),
            Err(e) => {
                crate::log::warn(&format!("couldn't install Dupin {}: {e}", staged.version));
                self.shared.lock().applied = false;
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

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn run(shared: &Shared, installed: &Installed, panel: &AppHandle<Ui>) {
    std::thread::sleep(FIRST_CHECK);
    loop {
        let idle = {
            let inner = shared.lock();
            if inner.applied {
                return;
            }
            inner.staged.is_none()
        };
        if idle && shared.enabled.load(Ordering::Relaxed) {
            match check(installed) {
                Ok(Some(staged)) => adopt(shared, staged, panel),
                Ok(None) => {}
                Err(e) => crate::log::warn(&format!("couldn't update: {e}")),
            }
        }
        std::thread::sleep(INTERVAL);
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
    inner.staged = Some(staged);
    panel.update(|s, _| s.updated());
}

/// Stage the feed's build if it is newer.
fn check(installed: &Installed) -> Result<Option<Staged>, String> {
    let feed = Feed::parse(&output(curl(Duration::from_secs(60)).arg(&installed.feed))?)?;
    if feed.build <= installed.build {
        return Ok(None);
    }
    stage(installed, &feed).map(Some)
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
    let mut command = Command::new("/usr/bin/curl");
    command
        .args(["--fail", "--silent", "--show-error", "--location"])
        .args(["--connect-timeout", "30", "--max-time"])
        .arg(max.as_secs().to_string());
    command
}

fn info_value(bundle: &Path, key: &str) -> Result<String, String> {
    let value = output(
        Command::new("/usr/bin/plutil")
            .args(["-extract", key, "raw", "-o", "-"])
            .arg(bundle.join("Contents/Info.plist")),
    )?;
    Ok(value.trim().to_owned())
}

/// Start the installed bundle once this process has exited, and exit.
fn relaunch(bundle: &Path) -> ! {
    let spawned = Command::new("/bin/sh")
        .args([
            "-c",
            "while /bin/kill -0 \"$1\" 2>/dev/null; do /bin/sleep 0.1; done; exec /usr/bin/open \"$2\"",
            "sh",
        ])
        .arg(std::process::id().to_string())
        .arg(bundle)
        .spawn();
    if let Err(e) = spawned {
        crate::log::warn(&format!("couldn't relaunch after the update: {e}"));
    }
    std::process::exit(0);
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
