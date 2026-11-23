//! The clipboard history's lifetime: kept in memory only, watched while `[clipboard]` is
//! enabled, and dropped as soon as it is turned off.

use std::sync::{Arc, Mutex};

use dupin_core::clipboard::{Access, Content, History, Policy};
use dupin_core::config;

pub struct Clipboard {
    history: Arc<Mutex<History>>,
    monitor: Option<dupin_mac::clipboard::Monitor>,
    /// A headless render's synthetic entries; the system clipboard is never read then.
    fixture: bool,
}

impl Clipboard {
    pub fn new(config: &config::Clipboard) -> Self {
        let mut clipboard = Self {
            history: Arc::new(Mutex::new(History::new(config.into()))),
            monitor: None,
            fixture: false,
        };
        if rune::tooling::is_headless() {
            clipboard.fixture = seed_fixture(&clipboard.history);
        }
        clipboard.apply(config);
        clipboard
    }

    pub fn history(&self) -> Arc<Mutex<History>> {
        self.history.clone()
    }

    /// Whether the history's contents or recording changed, so a list on screen is stale.
    pub fn apply(&mut self, config: &config::Clipboard) -> bool {
        let mut history = self.history.lock().unwrap();
        let policy = Policy::from(config);
        let mut changed = *history.policy() != policy;
        history.set_policy(policy);
        if config.enabled && !rune::tooling::is_headless() {
            if self.monitor.is_none() {
                changed = true;
                self.monitor = dupin_mac::clipboard::Monitor::start(self.history.clone())
                    .map_err(|error| crate::log::warn(&format!("couldn't watch the clipboard: {error}")))
                    .ok();
            }
        } else {
            changed |= self.monitor.take().is_some();
            if !self.fixture {
                history.clear();
                history.set_access(Access::default());
            }
        }
        changed
    }

    pub fn delete(&self, id: u64) {
        self.history.lock().unwrap().remove(id);
    }

    pub fn clear(&self) {
        self.history.lock().unwrap().clear();
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureEntry {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    files: Vec<std::path::PathBuf>,
    /// Seconds before the render.
    age: u64,
}

/// Seed the history from `DUPIN_HEADLESS_CLIPBOARD`, a JSON list of entries, oldest first.
fn seed_fixture(history: &Mutex<History>) -> bool {
    let Some(path) = std::env::var_os("DUPIN_HEADLESS_CLIPBOARD") else {
        return false;
    };
    let entries: Vec<FixtureEntry> = match std::fs::read_to_string(&path)
        .map_err(|error| error.to_string())
        .and_then(|source| serde_json::from_str(&source).map_err(|error| error.to_string()))
    {
        Ok(entries) => entries,
        Err(error) => {
            crate::log::warn(&format!("couldn't read the clipboard fixture: {error}"));
            return false;
        }
    };
    let now = dupin_core::session::unix_now();
    let mut history = history.lock().unwrap();
    for entry in entries {
        let content = match entry.text {
            Some(text) => Content::Text(text),
            None => Content::Files(entry.files),
        };
        history.push(content, now.saturating_sub(entry.age));
    }
    true
}
