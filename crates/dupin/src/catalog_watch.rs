use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use dupin_mac::{PathWatcher, watcher::canonical_root};

enum Event {
    Changed { root_moved: bool },
    Stop,
}

/// Runs scans and delivers their results on a worker. Dropping the watch stops it after
/// an in-progress scan delivers its result.
pub(crate) struct Watch {
    events: mpsc::Sender<Event>,
}

impl Watch {
    pub(crate) fn start<T>(
        directory: PathBuf,
        recursive: bool,
        scan: impl Fn(&Path, Option<String>) -> T + Send + 'static,
        deliver: impl Fn(T) + Send + 'static,
    ) -> io::Result<Self> {
        let (events, received) = mpsc::channel();
        let worker_events = events.clone();
        thread::Builder::new()
            .name("dupin-catalog-watch".into())
            .spawn(move || {
                let mut watcher = None;
                let mut watched = None;
                let mut root_moved = false;
                loop {
                    let requested = Roots::discover(&directory, recursive);
                    let mut watch_problem = None;
                    if root_moved || watched.as_ref() != Some(&requested) {
                        let events = worker_events.clone();
                        match PathWatcher::start_scoped(
                            requested.shallow.clone(),
                            requested.recursive.clone(),
                            move |root_moved| {
                                let _ = events.send(Event::Changed { root_moved });
                            },
                        ) {
                            Ok(next) => {
                                watcher.replace(next);
                                // A link may have moved between discovery and stream startup.
                                if Roots::discover(&directory, recursive) != requested {
                                    let _ = worker_events.send(Event::Changed { root_moved: false });
                                }
                                watched = Some(requested);
                            }
                            Err(error) => {
                                watched = None;
                                watch_problem = Some(format!(
                                    "Couldn't watch {} for changes: {error}",
                                    directory.display()
                                ));
                            }
                        }
                    }
                    deliver(scan(&directory, watch_problem));

                    let Ok(Event::Changed { root_moved: moved }) = received.recv() else {
                        break;
                    };
                    root_moved = moved;
                    while let Ok(event) = received.try_recv() {
                        match event {
                            Event::Changed { root_moved: moved } => root_moved |= moved,
                            Event::Stop => return,
                        }
                    }
                }
            })?;
        Ok(Self { events })
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        let _ = self.events.send(Event::Stop);
    }
}

#[derive(Default, PartialEq, Eq)]
struct Roots {
    shallow: Vec<PathBuf>,
    recursive: Vec<PathBuf>,
}

impl Roots {
    fn discover(directory: &Path, recursive: bool) -> Self {
        let mut roots = Self::default();
        let directory = std::path::absolute(directory).unwrap_or_else(|_| directory.to_owned());
        if let Ok(target) = roots.resolve(&directory)
            && target.is_dir()
        {
            let paths = if recursive {
                &mut roots.recursive
            } else {
                &mut roots.shallow
            };
            paths.push(canonical_root(&target));
            if let Ok(entries) = std::fs::read_dir(target) {
                for entry in entries.flatten() {
                    if entry.file_type().is_ok_and(|kind| kind.is_symlink())
                        && let Ok(target) = roots.resolve(&entry.path())
                        && recursive
                        && target.is_dir()
                    {
                        roots.recursive.push(canonical_root(&target));
                    }
                }
            }
        }
        roots.shallow.sort_unstable();
        roots.shallow.dedup();
        roots.recursive.sort_unstable();
        roots.recursive.dedup();
        roots
    }

    fn resolve(&mut self, path: &Path) -> io::Result<PathBuf> {
        crate::settings::resolve_with(path, |visited| {
            if let Some(parent) = visited.parent().filter(|parent| !parent.as_os_str().is_empty()) {
                self.shallow.push(canonical_root(parent));
            }
        })
    }
}
