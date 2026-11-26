//! What tells a watching worker that the files it read may have changed.

use std::path::PathBuf;

use dupin_mac::PathWatcher;

/// Called with `true` when a watched root itself was replaced or moved.
pub(crate) type Changed = Box<dyn Fn(bool) + Send + Sync>;

/// The files within the roots that the worker last read, whose in-place writes count.
pub(crate) type Files = Box<dyn Fn() -> Vec<PathBuf> + Send>;

/// The paths a watch covers.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Roots {
    /// A directory's entries, and writes to them.
    Entries(PathBuf),
    /// The entries of each `shallow` directory, and all of each `recursive` tree.
    Trees {
        shallow: Vec<PathBuf>,
        recursive: Vec<PathBuf>,
    },
}

/// What reports changes under the roots a worker asks for.
pub(crate) trait Source: Send + 'static {
    /// Reports changes under `roots` to `changed` until the returned value is dropped.
    fn watch(&mut self, roots: &Roots, files: Files, changed: Changed)
    -> Result<Box<dyn Send>, &'static str>;
}

/// FSEvents, reconciled against the files' metadata for what the daemon misses.
pub(crate) struct FsEvents;

impl Source for FsEvents {
    fn watch(
        &mut self,
        roots: &Roots,
        files: Files,
        changed: Changed,
    ) -> Result<Box<dyn Send>, &'static str> {
        let watcher = match roots {
            Roots::Entries(directory) => PathWatcher::start_entries(vec![directory.clone()], changed),
            Roots::Trees { shallow, recursive } => {
                PathWatcher::start_scoped(shallow.clone(), recursive.clone(), files, changed)
            }
        }?;
        Ok(Box::new(watcher))
    }
}

/// Changes a test reports by hand, so it waits on no daemon and no clock.
#[cfg(test)]
pub(crate) mod manual {
    use std::os::unix::fs::MetadataExt;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, Weak};

    use dupin_mac::watcher::canonical_root;

    use super::{Changed, Files, Roots, Source};

    #[derive(Clone, Copy)]
    enum Reach {
        /// The root and its direct entries.
        Entries,
        /// The root and everything below it.
        Tree,
    }

    struct Root {
        path: PathBuf,
        reach: Reach,
        /// Its device and inode when the watch started, if it existed.
        identity: Option<(u64, u64)>,
    }

    impl Root {
        fn reaches(&self, path: &Path) -> bool {
            self.path.starts_with(path)
                || match self.reach {
                    Reach::Entries => path.parent() == Some(self.path.as_path()),
                    Reach::Tree => path.starts_with(&self.path),
                }
        }
    }

    struct Watched {
        roots: Vec<Root>,
        changed: Changed,
    }

    /// A source that reports a change only when the test says one happened, and only to the
    /// watches whose roots FSEvents would report it under.
    #[derive(Clone, Default)]
    pub(crate) struct Manual {
        watches: Arc<Mutex<Vec<Weak<Watched>>>>,
    }

    fn identity(path: &Path) -> Option<(u64, u64)> {
        std::fs::metadata(path)
            .ok()
            .map(|metadata| (metadata.dev(), metadata.ino()))
    }

    impl Source for Manual {
        fn watch(
            &mut self,
            roots: &Roots,
            _files: Files,
            changed: Changed,
        ) -> Result<Box<dyn Send>, &'static str> {
            let reached = |path: &Path, reach| {
                let path = canonical_root(path);
                Root {
                    identity: identity(&path),
                    path,
                    reach,
                }
            };
            let roots = match roots {
                Roots::Entries(directory) => vec![reached(directory, Reach::Entries)],
                Roots::Trees { shallow, recursive } => shallow
                    .iter()
                    .map(|root| reached(root, Reach::Entries))
                    .chain(recursive.iter().map(|root| reached(root, Reach::Tree)))
                    .collect(),
            };
            let watched = Arc::new(Watched { roots, changed });
            let mut watches = self.watches.lock().unwrap();
            watches.retain(|watch| watch.strong_count() > 0);
            watches.push(Arc::downgrade(&watched));
            Ok(Box::new(watched))
        }
    }

    impl Manual {
        /// Reports that `path` was written, created, removed or renamed to every watch whose
        /// roots reach it, saying whether one of its roots is no longer the one it watched.
        /// Panics when no watch reaches it, since then no change there would ever be seen.
        pub(crate) fn changed(&self, path: &Path) {
            // As FSEvents reports it: through the real folders, but naming a link itself.
            let path = match (path.parent(), path.file_name()) {
                (Some(parent), Some(name)) => canonical_root(parent).join(name),
                _ => canonical_root(path),
            };
            let watches = self.watches.lock().unwrap();
            let mut reported = false;
            for watch in watches.iter().filter_map(Weak::upgrade) {
                if watch.roots.iter().any(|root| root.reaches(&path)) {
                    let moved = watch
                        .roots
                        .iter()
                        .any(|root| identity(&root.path) != root.identity);
                    (watch.changed)(moved);
                    reported = true;
                }
            }
            assert!(reported, "no watch reaches {}", path.display());
        }
    }
}
