//! The apps that run now, as NSWorkspace knows them, and hiding or quitting one.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use dupin_host_services::apps::{Apps, RunningApp};
use objc2::rc::autoreleasepool;
use objc2_app_kit::{NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace};

/// The bundle paths of every app that runs now, as their processes report them.
pub fn paths() -> HashSet<PathBuf> {
    autoreleasepool(|_| {
        NSWorkspace::sharedWorkspace()
            .runningApplications()
            .iter()
            .filter_map(|app| Some(PathBuf::from(app.bundleURL()?.path()?.to_string())))
            .collect()
    })
}

/// Whether the app bundle at `path` is among `running`, which [`paths`] read. A row's path can
/// be a link to where the bundle really is, as `/Applications/Safari.app` is.
pub fn is_running(running: &HashSet<PathBuf>, path: &Path) -> bool {
    running.contains(path) || std::fs::canonicalize(path).is_ok_and(|real| running.contains(&real))
}

/// The apps service's backend: NSWorkspace's regular apps, Dupin left out. Quitting goes
/// through `NSRunningApplication`, as the Dock's Quit and the Force Quit window do, so it needs
/// no Automation access.
pub struct RunningApps;

/// The regular app with this process, never Dupin itself.
fn regular(pid: u32) -> Option<objc2::rc::Retained<NSRunningApplication>> {
    if pid == std::process::id() {
        return None;
    }
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(i32::try_from(pid).ok()?)?;
    (app.activationPolicy() == NSApplicationActivationPolicy::Regular).then_some(app)
}

impl Apps for RunningApps {
    fn running(&self) -> Vec<RunningApp> {
        let own = std::process::id();
        autoreleasepool(|_| {
            NSWorkspace::sharedWorkspace()
                .runningApplications()
                .iter()
                .filter(|app| app.activationPolicy() == NSApplicationActivationPolicy::Regular)
                .filter_map(|app| {
                    let pid = u32::try_from(app.processIdentifier()).ok()?;
                    if pid == own || app.isTerminated() {
                        return None;
                    }
                    let path = app
                        .bundleURL()
                        .and_then(|url| url.path())
                        .map(|path| path.to_string());
                    Some(RunningApp {
                        name: app
                            .localizedName()
                            .map(|name| name.to_string())
                            .or_else(|| path.as_deref().and_then(bundle_name))?,
                        bundle_id: app.bundleIdentifier().map(|id| id.to_string()),
                        pid,
                        path,
                        active: app.isActive(),
                        hidden: app.isHidden(),
                    })
                })
                .collect()
        })
    }

    fn resolve(&self, path: &str) -> Option<String> {
        std::fs::canonicalize(path)
            .ok()?
            .into_os_string()
            .into_string()
            .ok()
    }

    fn hide(&self, pid: u32) -> bool {
        autoreleasepool(|_| regular(pid).is_some_and(|app| app.hide()))
    }

    fn quit(&self, pid: u32, force: bool) -> bool {
        autoreleasepool(|_| {
            regular(pid).is_some_and(|app| {
                if force {
                    app.forceTerminate()
                } else {
                    app.terminate()
                }
            })
        })
    }

    fn alive(&self, pid: u32) -> bool {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return false;
        };
        // SAFETY: signal 0 only checks that the process exists; nothing is sent.
        let sent = unsafe { libc::kill(pid, 0) } == 0;
        sent || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

/// `Name` for a bundle at `/…/Name.app`.
fn bundle_name(path: &str) -> Option<String> {
    Path::new(path)
        .file_stem()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_to_a_running_bundle_counts_as_running() {
        let folder = tempfile::tempdir().unwrap();
        let real = folder.path().join("Real.app");
        std::fs::create_dir(&real).unwrap();
        let link = folder.path().join("Link.app");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let running = HashSet::from([std::fs::canonicalize(&real).unwrap()]);
        assert!(is_running(&running, &link));
        assert!(!is_running(&running, &folder.path().join("Other.app")));
        assert!(!is_running(&HashSet::new(), &link));
    }
}
