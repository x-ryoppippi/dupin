use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use dupin_mac::watcher::canonical_root;

use crate::watch_source::{Roots, Source};

#[derive(Clone, Copy)]
pub(crate) enum Depth {
    Entries,
    Trees,
}

enum Event {
    Changed { root_moved: bool },
    Stop,
}

/// Runs scans and delivers updates on a worker. Dropping the watch lets an in-progress scan finish.
pub(crate) struct Watch {
    events: mpsc::Sender<Event>,
}

impl Watch {
    /// `scan` adds to its last argument the files within the trees its result was read from,
    /// so an in-place write to one is seen even if FSEvents never reports it.
    pub(crate) fn start<T>(
        directory: PathBuf,
        depth: Depth,
        mut source: impl Source,
        mut scan: impl FnMut(&Path, Option<String>, &mut Vec<PathBuf>) -> Option<T> + Send + 'static,
        deliver: impl Fn(T) + Send + 'static,
    ) -> io::Result<Self> {
        let (events, received) = mpsc::channel();
        let worker_events = events.clone();
        let files = Arc::new(Mutex::new(Vec::new()));
        thread::Builder::new()
            .name("dupin-catalog-watch".into())
            .spawn(move || {
                let mut watcher = None;
                let mut watched = None;
                let mut root_moved = false;
                loop {
                    let requested = discover(&directory, depth);
                    let mut watch_problem = None;
                    if root_moved || watched.as_ref() != Some(&requested) {
                        let events = worker_events.clone();
                        let files = files.clone();
                        match source.watch(
                            &requested,
                            Box::new(move || {
                                files
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .clone()
                            }),
                            Box::new(move |root_moved| {
                                let _ = events.send(Event::Changed { root_moved });
                            }),
                        ) {
                            Ok(next) => {
                                watcher.replace(next);
                                // A link may have moved between discovery and stream startup.
                                if discover(&directory, depth) != requested {
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
                    let mut read = Vec::new();
                    let result = scan(&directory, watch_problem, &mut read);
                    *files.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = read;
                    if let Some(result) = result {
                        deliver(result);
                    }

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

fn discover(directory: &Path, depth: Depth) -> Roots {
    match depth {
        Depth::Entries => Roots::Entries(directory.to_owned()),
        Depth::Trees => Trees::discover(directory).into(),
    }
}

#[derive(Default)]
struct Trees {
    shallow: Vec<PathBuf>,
    recursive: Vec<PathBuf>,
}

impl From<Trees> for Roots {
    fn from(trees: Trees) -> Self {
        Roots::Trees {
            shallow: trees.shallow,
            recursive: trees.recursive,
        }
    }
}

impl Trees {
    fn discover(directory: &Path) -> Self {
        let mut roots = Self::default();
        let directory = std::path::absolute(directory).unwrap_or_else(|_| directory.to_owned());
        if let Ok(target) = roots.resolve(&directory)
            && target.is_dir()
        {
            roots.recursive.push(canonical_root(&target));
            if let Ok(entries) = std::fs::read_dir(target) {
                for entry in entries.flatten() {
                    if entry.file_type().is_ok_and(|kind| kind.is_symlink())
                        && let Ok(target) = roots.resolve(&entry.path())
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
