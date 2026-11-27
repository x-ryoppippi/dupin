//! The `apps` capability: the apps that run now with a place in the Dock, and hiding or quitting
//! one of them.
//!
//! Only regular apps are listed, never agents, background processes or Dupin itself, and a
//! request reaches only an app the list holds, so a package can't hide or quit anything else.
//! Quitting asks the app as the Dock's Quit does, and Force Quit stops it as the Force Quit
//! window does; neither sends an Apple event of a package's own, so no Automation access is
//! needed. Dupin waits a moment for the app to end and says whether it did, never longer.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::service::bounded;

const MAX_NAME_BYTES: usize = 256;
const MAX_PATH_BYTES: usize = 4096;
/// How long Dupin waits for an app it asked to quit to end before saying it hasn't yet.
pub const QUIT_WAIT: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(50);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunningApp {
    pub name: String,
    #[serde(default)]
    pub bundle_id: Option<String>,
    pub pid: u32,
    /// The app bundle's absolute path, where macOS reports one.
    #[serde(default)]
    pub path: Option<String>,
    /// In front of every other app.
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub hidden: bool,
}

/// Which app a request names: by its process, or by its bundle's path, as an app row has it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Pid(u32),
    Path(String),
}

/// What hiding or quitting did to the app it reached.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Outcome {
    pub app: RunningApp,
    /// Hidden, or ended within [`QUIT_WAIT`].
    pub done: bool,
}

/// Where the apps come from: NSWorkspace in `dupin-mac`, [`Fixture`] in tests.
pub trait Apps: Send + Sync {
    /// The regular apps that run now, Dupin left out.
    fn running(&self) -> Vec<RunningApp>;
    /// `path` with its links resolved, as [`Apps::running`] reports a bundle's path.
    fn resolve(&self, path: &str) -> Option<String>;
    /// Whether the app took the request to hide.
    fn hide(&self, pid: u32) -> bool;
    /// Ask the app to quit, or with `force` stop it; whether the request reached it.
    fn quit(&self, pid: u32, force: bool) -> bool;
    /// Whether the process still runs.
    fn alive(&self, pid: u32) -> bool;
}

pub struct Service<A> {
    backend: A,
}

impl<A: Apps> Service<A> {
    pub fn new(backend: A) -> Self {
        Self { backend }
    }

    pub fn running(&self) -> Vec<RunningApp> {
        self.backend
            .running()
            .into_iter()
            .map(|app| RunningApp {
                name: bounded(app.name, MAX_NAME_BYTES),
                bundle_id: app.bundle_id.filter(|id| id.len() <= MAX_NAME_BYTES),
                path: app.path.filter(|path| {
                    path.starts_with('/')
                        && path.len() <= MAX_PATH_BYTES
                        && !path.chars().any(char::is_control)
                }),
                ..app
            })
            .collect()
    }

    /// The listed app `target` names, or none when no such app runs.
    fn find(&self, target: &Target) -> Option<RunningApp> {
        let running = self.running();
        match target {
            Target::Pid(pid) => running.into_iter().find(|app| app.pid == *pid),
            Target::Path(path) => {
                let resolved = self.backend.resolve(path);
                running.into_iter().find(|app| {
                    app.path
                        .as_deref()
                        .is_some_and(|bundle| bundle == path || resolved.as_deref() == Some(bundle))
                })
            }
        }
    }

    /// Hides the app, or none when no such app runs.
    pub fn hide(&self, target: &Target) -> Option<Outcome> {
        let app = self.find(target)?;
        let done = self.backend.hide(app.pid);
        Some(Outcome { app, done })
    }

    /// Asks the app to quit, or with `force` stops it, and waits up to [`QUIT_WAIT`] for it to
    /// end; none when no such app runs. An app that is asking to save, or refuses, isn't done.
    pub fn quit(&self, target: &Target, force: bool) -> Option<Outcome> {
        let app = self.find(target)?;
        let done = self.backend.quit(app.pid, force) && self.ended(app.pid);
        Some(Outcome { app, done })
    }

    fn ended(&self, pid: u32) -> bool {
        let deadline = Instant::now() + QUIT_WAIT;
        loop {
            if !self.backend.alive(pid) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(POLL);
        }
    }
}

/// A fixed list of running apps: hiding one marks it hidden, quitting ends it unless it is
/// among `refuse`, which only a forced quit ends.
#[derive(Debug, Default)]
pub struct Fixture {
    apps: std::sync::Mutex<Vec<RunningApp>>,
    refuse: Vec<u32>,
}

impl Fixture {
    pub fn new(apps: Vec<RunningApp>, refuse: Vec<u32>) -> Self {
        Self {
            apps: std::sync::Mutex::new(apps),
            refuse,
        }
    }

    fn apps(&self) -> std::sync::MutexGuard<'_, Vec<RunningApp>> {
        self.apps
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Apps for Fixture {
    fn running(&self) -> Vec<RunningApp> {
        self.apps().clone()
    }

    fn resolve(&self, _path: &str) -> Option<String> {
        None
    }

    fn hide(&self, pid: u32) -> bool {
        let mut apps = self.apps();
        let Some(app) = apps.iter_mut().find(|app| app.pid == pid) else {
            return false;
        };
        app.hidden = true;
        app.active = false;
        true
    }

    fn quit(&self, pid: u32, force: bool) -> bool {
        if force || !self.refuse.contains(&pid) {
            self.apps().retain(|app| app.pid != pid);
        }
        true
    }

    fn alive(&self, pid: u32) -> bool {
        self.apps().iter().any(|app| app.pid == pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, pid: u32, path: &str) -> RunningApp {
        RunningApp {
            name: name.into(),
            bundle_id: Some(format!("com.example.{name}")),
            pid,
            path: Some(path.into()),
            active: false,
            hidden: false,
        }
    }

    fn service(refuse: Vec<u32>) -> Service<Fixture> {
        Service::new(Fixture::new(
            vec![
                app("Notes", 10, "/Applications/Notes.app"),
                app("Editor", 11, "/Applications/Editor.app"),
            ],
            refuse,
        ))
    }

    #[test]
    fn a_request_reaches_only_a_listed_app_by_its_process_or_its_path() {
        let service = service(Vec::new());
        assert_eq!(service.hide(&Target::Pid(99)), None, "an unlisted process");
        assert_eq!(
            service.quit(&Target::Path("/Applications/Other.app".into()), true),
            None
        );
        let hidden = service
            .hide(&Target::Path("/Applications/Notes.app".into()))
            .unwrap();
        assert_eq!((hidden.app.pid, hidden.done), (10, true));
        assert!(service.running()[0].hidden);
        let quit = service.quit(&Target::Pid(11), false).unwrap();
        assert_eq!((quit.app.name.as_str(), quit.done), ("Editor", true));
        assert_eq!(service.running().len(), 1);
    }

    #[test]
    fn an_app_that_refuses_to_quit_is_reported_after_the_wait_and_force_ends_it() {
        let service = service(vec![10]);
        let started = Instant::now();
        let refused = service.quit(&Target::Pid(10), false).unwrap();
        assert!(!refused.done);
        assert!(started.elapsed() >= QUIT_WAIT && started.elapsed() < QUIT_WAIT * 2);
        assert!(service.quit(&Target::Pid(10), true).unwrap().done);
        assert_eq!(service.quit(&Target::Pid(10), true), None, "it no longer runs");
    }

    #[test]
    fn names_are_bounded_and_a_path_that_isnt_absolute_is_dropped() {
        let mut long = app(&"x".repeat(MAX_NAME_BYTES + 5), 1, "relative.app");
        long.bundle_id = Some("b".repeat(MAX_NAME_BYTES + 1));
        let listed = Service::new(Fixture::new(vec![long], Vec::new())).running();
        assert_eq!(listed[0].name.len(), MAX_NAME_BYTES);
        assert_eq!((&listed[0].bundle_id, &listed[0].path), (&None, &None));
    }
}
