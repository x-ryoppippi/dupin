//! The clipboard history's lifetime: kept in memory only, watched while `[clipboard]` is
//! enabled, and dropped as soon as it is turned off.

use std::sync::{Arc, Mutex};

use dupin_core::clipboard::{Access, Content, History, Policy};
use dupin_core::config;

pub struct Clipboard {
    history: Arc<Mutex<History>>,
    monitor: Option<dupin_mac::clipboard::Monitor>,
    enabled: bool,
}

impl Clipboard {
    pub fn new(config: &config::Clipboard) -> Self {
        let mut clipboard = Self {
            history: Arc::new(Mutex::new(History::new(config.into()))),
            monitor: None,
            enabled: false,
        };
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
        let mut changed = self.enabled != config.enabled || *history.policy() != policy;
        history.set_policy(policy);
        if !config.enabled {
            self.monitor.take();
            history.clear();
            history.set_access(Access::default());
        } else if rune::tooling::is_headless() {
            if !self.enabled {
                seed_fixture(&mut history);
            }
        } else if self.monitor.is_none() {
            changed = true;
            self.monitor = dupin_mac::clipboard::Monitor::start(self.history.clone())
                .map_err(|error| crate::log::warn(&format!("couldn't watch the clipboard: {error}")))
                .ok();
        }
        self.enabled = config.enabled;
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
fn seed_fixture(history: &mut History) {
    let Some(path) = std::env::var_os("DUPIN_HEADLESS_CLIPBOARD") else {
        return;
    };
    let entries: Vec<FixtureEntry> = match std::fs::read_to_string(&path)
        .map_err(|error| error.to_string())
        .and_then(|source| serde_json::from_str(&source).map_err(|error| error.to_string()))
    {
        Ok(entries) => entries,
        Err(error) => {
            crate::log::warn(&format!("couldn't read the clipboard fixture: {error}"));
            return;
        }
    };
    let now = dupin_core::session::unix_now();
    for entry in entries {
        let content = match entry.text {
            Some(text) => Content::Text(text),
            None => Content::Files(entry.files),
        };
        history.push(content, now.saturating_sub(entry.age));
    }
}
