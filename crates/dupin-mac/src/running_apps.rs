//! The apps that run now, as NSWorkspace knows them.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use objc2::rc::autoreleasepool;
use objc2_app_kit::NSWorkspace;

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
