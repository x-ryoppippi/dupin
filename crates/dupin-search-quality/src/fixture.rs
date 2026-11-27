//! What the benchmark searches and what it looks for: `scripts/search-quality/corpus.toml`
//! and `scripts/search-quality/cases.toml`.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The time every case runs at, in Unix seconds; the ages of picks count back from it.
pub const NOW: u64 = 1_800_000_000;
const HOUR: u64 = 60 * 60;
const DAY: u64 = 24 * HOUR;

/// The history a case runs with when it names none.
const BASE: &str = "base";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Corpus {
    /// App names, by the folder their bundles are in.
    apps: BTreeMap<String, Vec<String>>,
    /// The other names Spotlight knows an app by, by the name Finder shows.
    #[serde(default)]
    names: BTreeMap<String, Vec<String>>,
    /// Commands a keyword enters.
    #[serde(default)]
    pub commands: Vec<Command>,
    /// Web searches' names, by alias.
    #[serde(default)]
    pub web: BTreeMap<String, String>,
    /// Apps, by the alias in `[aliases]` that reaches them.
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
    /// Picks, by the name of the history they make up. `base` is a heavy user's.
    #[serde(default)]
    history: BTreeMap<String, Vec<Picks>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub id: String,
    pub title: String,
    pub keywords: Vec<String>,
}

/// `times` picks of `app` after typing `query`, the latest `days` ago and each earlier one
/// `every_hours` before the next.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Picks {
    app: String,
    #[serde(default)]
    query: String,
    times: u32,
    #[serde(default)]
    days: u64,
    #[serde(default = "every_hours")]
    every_hours: u64,
}

fn every_hours() -> u64 {
    3
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cases {
    pub cases: Vec<Case>,
}

/// What a person wants, the history they have, and what they type for it, one character at a
/// time.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    /// The row wanted: `app:<name>`, `command:<id>`, `web:<alias>` or `system:<title>`.
    pub intent: String,
    pub query: String,
    #[serde(default = "base")]
    pub history: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn base() -> Vec<String> {
    vec![BASE.into()]
}

/// One app: where its bundle is, the name Finder shows, and its other names.
pub type App = (PathBuf, String, Vec<String>);

/// A pick: when, the ID of what was opened, and the query typed for it.
pub type Pick = (u64, String, String);

impl Corpus {
    pub fn parse(text: &str) -> Result<Self, String> {
        let corpus: Self = toml::from_str(text).map_err(|error| format!("corpus: {error}"))?;
        corpus.check()?;
        Ok(corpus)
    }

    fn check(&self) -> Result<(), String> {
        let mut seen = HashSet::new();
        for name in self.apps.values().flatten() {
            if !seen.insert(name.as_str()) {
                return Err(format!("corpus: the app {name} is listed twice"));
            }
        }
        for name in self.names.keys().chain(self.aliases.values()) {
            self.app_path(name)?;
        }
        for picks in self.history.values().flatten() {
            self.app_path(&picks.app)?;
            if picks.times == 0 {
                return Err(format!("corpus: a pick of {} happens no times", picks.app));
            }
        }
        if !self.history.contains_key(BASE) {
            return Err(format!("corpus: no history is called {BASE}"));
        }
        Ok(())
    }

    /// Every app, in the order the folders and names are listed.
    pub fn apps(&self) -> Vec<App> {
        self.apps
            .iter()
            .flat_map(|(folder, names)| {
                names.iter().map(move |name| {
                    let extra = self.names.get(name).cloned().unwrap_or_default();
                    (bundle(folder, name), name.clone(), extra)
                })
            })
            .collect()
    }

    /// The ID the row for the app called `name` has: its bundle's path.
    pub fn app_path(&self, name: &str) -> Result<PathBuf, String> {
        self.apps
            .iter()
            .find(|(_, names)| names.iter().any(|listed| listed == name))
            .map(|(folder, _)| bundle(folder, name))
            .ok_or_else(|| format!("corpus: no app is called {name}"))
    }

    /// The picks of the histories `names`, oldest first.
    pub fn picks(&self, names: &[String]) -> Result<Vec<Pick>, String> {
        let mut picks = Vec::new();
        for name in names {
            let listed = self
                .history
                .get(name)
                .ok_or_else(|| format!("no history is called {name}"))?;
            for spec in listed {
                let id = self.app_path(&spec.app)?.to_string_lossy().into_owned();
                let latest = NOW - spec.days * DAY;
                for time in 0..u64::from(spec.times) {
                    picks.push((
                        latest - time * spec.every_hours * HOUR,
                        id.clone(),
                        spec.query.clone(),
                    ));
                }
            }
        }
        picks.sort();
        Ok(picks)
    }
}

fn bundle(folder: &str, name: &str) -> PathBuf {
    PathBuf::from(folder).join(format!("{name}.app"))
}

impl Cases {
    pub fn parse(text: &str) -> Result<Self, String> {
        let cases: Self = toml::from_str(text).map_err(|error| format!("cases: {error}"))?;
        let mut seen = HashSet::new();
        for case in &cases.cases {
            if !seen.insert(case.id.as_str()) {
                return Err(format!("cases: {} is listed twice", case.id));
            }
            if case.query.is_empty() {
                return Err(format!("cases: {} types nothing", case.id));
            }
        }
        Ok(cases)
    }
}
