//! Extensions, each routed by one keyword: those bundled with Dupin and the user's own in
//! `extensions/<name>/` beside config.toml. Each is a JavaScript package compiled and run by
//! QuickJS-ng in the sandboxed extension host process. Guests get the query and return rows, or
//! hand over a catalogue Dupin matches itself. They reach host services only through the broker
//! bridge, which checks each request against the scopes a package declares and the user granted.

pub mod access;
pub mod bundled;
mod catalogue;
mod fixtures;
pub mod grants;
pub mod installed;
#[cfg(test)]
mod items_tests;
mod manifest;
#[cfg(test)]
mod overrides;
pub mod packages;
#[cfg(test)]
pub(crate) mod parity;
pub(crate) mod preferences;
mod provider;
#[cfg(test)]
mod remote_ssh;

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Instant, SystemTime};

use dupin_core::{Batch, Item};
use dupin_extension_bridge::{Call, Capabilities, PackageId, Revision};
use dupin_extension_host::{Compiled, Host, Services, Trust};
use dupin_js_runtime::web_search::WebSearch;
use dupin_js_runtime::{
    Acted, CataloguePage, Diagnostic, Exports, Handled, PageContext, Preferences, Reason, Row, RunError,
    Sources, ViewAnswer,
};

use catalogue::Asking;

pub use fixtures::{Rendered, render, test};
pub use provider::{ACTION_DEADLINE, ExtensionProvider, QUERY_DEADLINE, Role, answer};
pub(crate) use provider::{hand_over_kinds, is_answered_consent_row, on_catalogue_refreshed, stands_in};

pub(crate) use crate::catalog_watch::Watch;

const MAX_EXTENSIONS: usize = 64;

pub struct Extension {
    pub id: String,
    pub folder_name: String,
    pub name: String,
    pub version: String,
    pub keyword: String,
    /// The package's folder, which row icons are relative to.
    pub root: PathBuf,
    /// The package's own icon: its keyword chip's, its heading's in Settings and that of each
    /// row that names none.
    pub icon: Option<dupin_core::Icon>,
    /// The package asked for its catalogue to answer the default search too.
    pub default_search: bool,
    /// The package's `search` answers its keyword before anything is typed.
    pub empty_query: bool,
    /// How its catalogue is matched, which also decides whether picks rank it.
    pub matching: dupin_core::catalogue::Matching,
    preferences: preferences::Preferences,
    /// What the package exports, once a call has linked it.
    exports: OnceLock<Exports>,
    catalogue: catalogue::Cached,
    web_searches_noted: AtomicBool,
    /// A thread waits to search again once the catalogue outlives its `ttl`.
    waking: AtomicBool,
    /// What the broker knows the package by: its `name`, or `bundled:<name>` for one that
    /// ships with Dupin.
    pub package: PackageId,
    /// The revision grants are for: package.json and every module Dupin runs.
    pub revision: Revision,
    /// The most the package may do, as the broker read it from package.json.
    pub capabilities: Capabilities,
    /// The filters and kind actions it offers `[advanced]`.
    pub provides: manifest::Provides,
    /// The bytecode the extension host compiled, which only the host ever loads.
    compiled: Compiled,
}

/// Which helper asks a package for its catalogue: the resident one a search waits for, or one
/// of its own for a refresh nobody waits for, which may run a program for as long as it takes
/// without holding up the searches of every package that shares the resident one.
#[derive(Clone, Copy)]
enum Fetching {
    Resident,
    Alone,
}

/// Where an extension was found. A user extension with a bundled one's `name` replaces it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Bundled,
    User,
}

impl std::fmt::Debug for Extension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Extension")
            .field("id", &self.id)
            .field("version", &self.version)
            .field("keyword", &self.keyword)
            .finish_non_exhaustive()
    }
}

impl Extension {
    /// Whether an `enter` effect of this package may hand its query to the command `command`
    /// names. The command's package receives the text, so only a package that ships with Dupin
    /// may name another: any other could pass what it read to one allowed to send it elsewhere.
    pub(crate) fn may_enter(&self, command: &str) -> bool {
        command == self.id || self.package.as_str().starts_with(BUNDLED_PREFIX)
    }

    /// Links the package and reports which of `search` and `items` its entry exports, which
    /// must be at least one.
    pub fn check(&self, deadline: Instant) -> Result<Exports, RunError> {
        if let Some(exports) = self.exports.get() {
            return Ok(*exports);
        }
        let exports = host().check(&self.compiled, deadline, &|| false)?;
        Ok(*self.exports.get_or_init(|| exports))
    }

    /// Asks the guest for `query`, answering its host requests with `services`.
    pub fn query(
        &self,
        query: &str,
        limit: usize,
        deadline: Instant,
        is_cancelled: impl Fn() -> bool + 'static,
        services: &Arc<dyn Services>,
    ) -> Result<Vec<Row>, RunError> {
        host().search(&self.compiled, query, limit, deadline, &is_cancelled, services)
    }

    /// Runs `filters[name]` on one batch of a source it is composed onto. It reaches no host
    /// service, so it has no external effects or persistent state.
    pub fn filter(
        &self,
        name: &str,
        input: &dupin_js_runtime::FilterInput,
        deadline: Instant,
    ) -> Result<Vec<dupin_js_runtime::FilterHit>, RunError> {
        let services: Arc<dyn Services> = Arc::new(dupin_extension_host::NoServices);
        host().filter(
            &self.compiled,
            name,
            input,
            &self.preferences(),
            deadline,
            &|| false,
            &services,
        )
    }

    /// Asks the guest's `views[name]` for the page `page` describes, answering its host
    /// requests with `services`.
    pub fn view(
        &self,
        name: &str,
        page: &PageContext,
        deadline: Instant,
        is_cancelled: impl Fn() -> bool + 'static,
        services: &Arc<dyn Services>,
    ) -> Result<ViewAnswer, RunError> {
        host().view(
            &self.compiled,
            name,
            page,
            &self.preferences(),
            deadline,
            &is_cancelled,
            services,
        )
    }

    /// Runs `actions[name](data)`, which the user chose from one of the package's rows on
    /// `page`, and asks for the catalogue again if the handler invalidated it.
    pub fn act(
        &self,
        name: &str,
        data: &serde_json::Value,
        page: &PageContext,
        deadline: Instant,
        services: &Arc<dyn Services>,
    ) -> Result<Acted, RunError> {
        let acted = host().action(
            &self.compiled,
            name,
            data,
            page,
            &self.preferences(),
            deadline,
            &|| false,
            services,
        )?;
        if acted.invalidated {
            self.catalogue.invalidate();
        }
        Ok(acted)
    }

    /// Runs `events[name](event)` for something Dupin watched on the package's behalf, and
    /// asks for the catalogue again if the handler invalidated it.
    pub fn event(
        &self,
        name: &str,
        event: &serde_json::Value,
        deadline: Instant,
        services: &Arc<dyn Services>,
    ) -> Result<Handled, RunError> {
        let handled = host().event(
            &self.compiled,
            name,
            event,
            &self.preferences(),
            deadline,
            &|| false,
            services,
        )?;
        if handled.invalidated {
            self.catalogue.invalidate();
        }
        Ok(handled)
    }

    /// The catalogue's rows for `query`, asking the package for its catalogue first when Dupin
    /// holds none, it went stale, its preferences changed or it was made under another
    /// generation than `call`'s. Its host requests go through `call`, as a search's do. Typing
    /// never cancels that, since the next keystroke needs the same catalogue. Every part of it
    /// must arrive before `deadline`, the search's own, so a slow catalogue fails with its own
    /// diagnostic before the search expires. For the main search, `role` holds off asking again
    /// after a failure. With the `refresher` the launcher holds the package by, a catalogue that
    /// has only outlived its `ttl` answers from the rows held while a thread of its own asks the
    /// package again.
    pub fn catalogue(
        &self,
        query: &str,
        limit: usize,
        deadline: Instant,
        call: &Call,
        role: Role,
        refresher: Option<&Arc<Extension>>,
    ) -> Result<Vec<Batch>, RunError> {
        let settings = self.settings();
        let asking = Asking {
            preferences: &settings,
            generation: call.generation,
            role,
        };
        if let Some(refresher) = refresher
            && self.catalogue.stale(asking) == Some(true)
        {
            refresher.refresh_in_background(settings.clone(), call);
        }
        let never = || false;
        let rows = self
            .catalogue
            .search(
                query,
                limit,
                asking,
                &mut self.fetch(host(), Fetching::Resident, deadline, &never, &call.services),
                self,
            )
            .map_err(|error| self.located(error))?;
        if let Some(refresher) = refresher {
            refresher.wake_when_stale();
        }
        Ok(rows)
    }

    /// Searches again, through [`on_catalogue_refreshed`], once the rows just listed outlive
    /// their `ttl`, so a catalogue the panel goes on listing is asked for again without a
    /// keystroke. That search finds it stale and refreshes it in the background, and the
    /// refresh searches again once it lands; when the panel no longer lists the catalogue, the
    /// launcher doesn't search, and nothing wakes it until it is next listed.
    fn wake_when_stale(self: &Arc<Self>) {
        let Some(expires) = self.catalogue.expires() else {
            return;
        };
        if self.waking.swap(true, Ordering::AcqRel) {
            return;
        }
        let extension = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name(format!("catalogue-wake-{}", self.id))
            .spawn(move || {
                std::thread::sleep(expires.saturating_duration_since(Instant::now()));
                extension.waking.store(false, Ordering::Release);
                provider::catalogue_refreshed(&extension.package);
            });
        if spawned.is_err() {
            self.waking.store(false, Ordering::Release);
        }
    }

    /// Asks the package again for its stale catalogue on a thread of its own, through `call`'s
    /// services and under its generation, as the search that found it stale would have.
    fn refresh_in_background(self: &Arc<Self>, settings: Arc<Preferences>, call: &Call) {
        let extension = Arc::clone(self);
        let services = call.services.clone();
        let generation = call.generation;
        let spawned = std::thread::Builder::new()
            .name(format!("catalogue-{}", self.id))
            .spawn(move || {
                let never = || false;
                let deadline = Instant::now() + QUERY_DEADLINE;
                let refreshed = extension.catalogue.refresh_stale(
                    &settings,
                    generation,
                    &mut extension.fetch(host(), Fetching::Alone, deadline, &never, &services),
                    &*extension,
                );
                if refreshed {
                    provider::catalogue_refreshed(&extension.package);
                }
            });
        if let Err(error) = spawned {
            crate::log::warn(&format!(
                "extension {}: couldn't refresh its catalogue: {error}",
                self.id
            ));
            self.catalogue.forget();
        }
    }

    /// Asks `host` for the parts of the package's catalogue. Every part of one catalogue is
    /// asked for with the same settings, which may hold thousands of snippets, so their
    /// preferences are encoded once, and again only for other settings.
    fn fetch<'a>(
        &'a self,
        host: &'a Host,
        fetching: Fetching,
        deadline: Instant,
        cancelled: &'a dyn Fn() -> bool,
        services: &'a Arc<dyn Services>,
    ) -> impl FnMut(Option<&str>, Reason, &Arc<Preferences>) -> Result<CataloguePage, RunError> + 'a {
        let mut encoded: Option<(Arc<Preferences>, dupin_extension_host::Preferences)> = None;
        move |cursor, reason, settings| {
            let held = encoded.take().filter(|(held, _)| Arc::ptr_eq(held, settings));
            let (_, preferences) = encoded.insert(held.unwrap_or_else(|| {
                let preferences = self.preferences_with(settings);
                (
                    settings.clone(),
                    dupin_extension_host::Preferences::encode(&preferences),
                )
            }));
            let items = match fetching {
                Fetching::Resident => Host::items,
                Fetching::Alone => Host::items_alone,
            };
            items(
                host,
                &self.compiled,
                cursor,
                reason,
                preferences,
                deadline,
                cancelled,
                services,
            )
        }
    }

    /// The catalogue's web searches, each with the row it shows before a query is typed, asking
    /// the package first as [`Extension::catalogue`] does.
    pub fn web_searches(
        &self,
        deadline: Instant,
        call: &Call,
        role: Role,
    ) -> Result<Vec<(Item, WebSearch)>, RunError> {
        let settings = self.settings();
        let asking = Asking {
            preferences: &settings,
            generation: call.generation,
            role,
        };
        let never = || false;
        self.catalogue
            .web_searches(
                asking,
                &mut self.fetch(host(), Fetching::Resident, deadline, &never, &call.services),
                self,
            )
            .map_err(|error| self.located(error))
    }

    /// What [`Extension::web_searches`] would return, if Dupin can answer it now without asking
    /// the package or waiting.
    pub fn held_web_searches(&self) -> Option<Vec<(Item, WebSearch)>> {
        if !self.exports.get()?.items {
            return Some(Vec::new());
        }
        self.catalogue.held_web_searches(&self.settings())
    }

    /// Whether the row `id` is one of this package's. A built-in's port keeps the built-in's
    /// row IDs, which name no package, so it owns none by ID.
    pub fn owns_row(&self, id: &str) -> bool {
        let prefix = catalogue::Convert::entry_id(self, "");
        !prefix.is_empty() && id.starts_with(&prefix)
    }

    /// Logs, once, that the package's web searches reach only its own keyword.
    fn note_keyword_only_web_searches(&self) {
        if crate::web::reaches_main_search(&self.package)
            || self.web_searches_noted.swap(true, Ordering::Relaxed)
        {
            return;
        }
        crate::log::warn(&format!(
            "extension {}: its web searches are listed only under its keyword `{}`; only the bundled Web Search package adds aliases and fallbacks to the main search",
            self.id, self.keyword
        ));
    }

    /// Drops the catalogue Dupin holds, so its next use asks for it from the start.
    pub fn forget_catalogue(&self) {
        self.catalogue.forget();
    }

    /// What `ctx.preferences` holds for the package's calls.
    pub fn preferences(&self) -> serde_json::Value {
        self.preferences_with(&self.settings())
    }

    /// The config.toml settings the package is handed, or in a render of one of its fixtures,
    /// the fixture's preferences.
    fn settings(&self) -> Arc<Preferences> {
        fixtures::headless()
            .and_then(|fixture| fixture.preferences(self))
            .unwrap_or_else(|| preferences::of(&self.package))
    }

    /// The package's declared preferences with `settings`, a built-in's config.toml settings
    /// for the package Dupin shipped for it.
    fn preferences_with(&self, settings: &Preferences) -> serde_json::Value {
        let mut values = self.preferences.values(&self.id);
        if let serde_json::Value::Object(values) = &mut values {
            values.extend(settings.clone());
        }
        values
    }

    /// A catalogue's failure, named after the package's entry when nothing else locates it.
    fn located(&self, error: RunError) -> RunError {
        match error {
            RunError::Failed(diagnostic) if diagnostic.file.is_none() => {
                RunError::Failed(diagnostic.file(self.compiled.entry()))
            }
            error => error,
        }
    }
}

/// Starts the copies of the fixture `dupin ext render` renders, once the headless launcher's
/// first frame starts its script.
pub(crate) fn headless_ready() {
    if let Some(fixture) = fixtures::headless() {
        fixture.play_events();
    }
}

/// The one extension host every JavaScript extension shares, which starts a trust domain's
/// helper on that domain's first query. Every bundled package's calls queue for one helper, and
/// the wait counts against each call's deadline.
fn host() -> &'static Host {
    launcher_wide!(
        Host,
        Host::fixed(Box::new(helper_command), headless_environment())
    )
}

/// Where the launcher reads the time it measures guests' timestamps against, such as how long
/// ago a catalogue row is from: the clock its guests see, so a headless run shows the same any
/// day, or else the wall clock.
#[derive(Clone, Copy)]
pub(crate) struct Clock {
    fixed: Option<i64>,
    /// Unix milliseconds.
    wall: fn() -> u64,
}

impl Clock {
    pub(crate) fn launcher() -> Self {
        Self {
            fixed: headless_environment().map(|environment| environment.now),
            wall: || dupin_core::session::unix_now() * 1000,
        }
    }

    /// One that stands at `now`, in Unix milliseconds.
    pub(crate) fn fixed(now: i64) -> Self {
        Self {
            fixed: Some(now),
            ..Self::launcher()
        }
    }

    /// Unix milliseconds.
    pub(crate) fn now_ms(self) -> u64 {
        self.fixed
            .map_or_else(self.wall, |now| u64::try_from(now).unwrap_or(0))
    }
}

/// The clock and seed of the fixture `dupin ext render` renders, which it hands the headless
/// launcher it starts, or else the clock of the headless calendar fixtures, so guests see the
/// same day the fixture events are laid out around. A launcher with a window never reads it.
fn headless_environment() -> Option<dupin_js_runtime::Environment> {
    static ENVIRONMENT: OnceLock<Option<dupin_js_runtime::Environment>> = OnceLock::new();
    *ENVIRONMENT.get_or_init(read_headless_environment)
}

fn read_headless_environment() -> Option<dupin_js_runtime::Environment> {
    if !rune::tooling::is_headless() {
        return None;
    }
    let Ok(value) = std::env::var(fixtures::HEADLESS_ENVIRONMENT) else {
        return Some(dupin_js_runtime::Environment {
            now: dupin_mac::FIXTURE_CLOCK,
            seed: 1,
        });
    };
    serde_json::from_str::<dupin_js_runtime::Environment>(&value)
        .ok()
        .filter(dupin_js_runtime::Environment::is_valid)
}

/// Dupin's own executable as the extension host, with nothing inherited it doesn't need. A
/// headless run's guests keep to the calendar fixtures' UTC unless it names a zone.
fn helper_command() -> std::io::Result<std::process::Command> {
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command.env_clear().current_dir("/");
    match std::env::var_os("TZ") {
        Some(zone) => {
            command.env("TZ", zone);
        }
        None if rune::tooling::is_headless() => {
            command.env("TZ", "UTC");
        }
        None => {}
    }
    #[cfg(not(test))]
    command.arg(dupin_extension_host::FLAG);
    #[cfg(test)]
    command
        .args([
            "--exact",
            "extensions::tests::extension_host",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(tests::EXTENSION_HOST_ENV, "1");
    Ok(command)
}

/// What a scan of the extensions folder found: the valid extensions, sorted by folder name,
/// and one line for each folder that was skipped.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub extensions: Vec<Arc<Extension>>,
    pub problems: Vec<String>,
}

/// `$XDG_CONFIG_HOME/dupin/extensions`, beside config.toml.
pub fn directory() -> Option<PathBuf> {
    Some(crate::settings::path()?.parent()?.join("extensions"))
}

/// Why a folder must stay off: it no longer matches the installed extensions list, or isn't a
/// bundled package this build sealed.
type Integrity = Box<dyn FnMut(&Path) -> Option<String> + Send>;

#[derive(Default)]
struct Scanner {
    loaded: HashMap<PathBuf, LoadedExtension>,
    previous: Option<Catalog>,
    integrity: Option<Integrity>,
    /// Whether the folder holds the packages that ship with Dupin.
    bundled: bool,
    /// The package folders the last scan read.
    folders: Vec<PathBuf>,
}

impl Scanner {
    fn update(&mut self, directory: &Path, watch_problem: Option<String>) -> Option<Catalog> {
        let mut catalog = self.scan(directory);
        catalog.problems.extend(watch_problem);
        if self.previous.as_ref().is_some_and(|previous| {
            previous.problems == catalog.problems
                && previous.extensions.len() == catalog.extensions.len()
                && previous
                    .extensions
                    .iter()
                    .zip(&catalog.extensions)
                    .all(|(old, new)| Arc::ptr_eq(old, new))
        }) {
            return None;
        }
        self.previous = Some(catalog.clone());
        Some(catalog)
    }

    /// The files the last scan read packages from: each folder's package.json, and the modules
    /// and icon of each package it loaded or, of one it couldn't load, the modules its manifest
    /// leads to.
    fn files(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        for folder in &self.folders {
            files.push(folder.join(manifest::PACKAGE_JSON));
            match self.loaded.get(folder) {
                Some(loaded) => {
                    files.extend(loaded.files.iter().map(|file| file.path.clone()));
                    files.extend(loaded.icon.as_ref().map(|icon| icon.stamp.path.clone()));
                }
                None => {
                    let sources = manifest::Stamps::read(folder)
                        .and_then(|stamps| manifest::read(&stamps))
                        .and_then(|manifest| dupin_js_runtime::source_files(folder, &manifest.entry));
                    files.extend(sources.into_iter().flatten().map(|(_, path)| path));
                }
            }
        }
        files
    }

    fn scan(&mut self, directory: &Path) -> Catalog {
        let mut previous = std::mem::take(&mut self.loaded);
        self.folders.clear();
        let mut catalog = Catalog::default();
        let entries = match std::fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return catalog,
            Err(error) => {
                catalog
                    .problems
                    .push(format!("Couldn't read {}: {error}", directory.display()));
                return catalog;
            }
        };
        let mut folders: Vec<_> = entries
            .filter_map(Result::ok)
            .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        folders.sort();
        if folders.len() > MAX_EXTENSIONS {
            catalog.problems.push(format!(
                "Only the first {MAX_EXTENSIONS} extension folders were read."
            ));
            folders.truncate(MAX_EXTENSIONS);
        }
        self.folders.clone_from(&folders);
        for folder in folders {
            if let Some(problem) = self.integrity.as_mut().and_then(|integrity| integrity(&folder)) {
                catalog
                    .problems
                    .push(format!("{}: {problem}", folder_name(&folder)));
                continue;
            }
            match load(&folder, previous.remove(&folder), self.bundled) {
                Ok(loaded) => {
                    let extension = &loaded.extension;
                    if catalog.extensions.iter().any(|e| e.id == extension.id) {
                        catalog.problems.push(format!(
                            "{}: id `{}` is used by another extension",
                            extension.folder_name, extension.id
                        ));
                    } else {
                        catalog.extensions.push(extension.clone());
                    }
                    self.loaded.insert(folder, loaded);
                }
                Err(problem) => catalog
                    .problems
                    .push(format!("{}: {problem}", folder_name(&folder))),
            }
        }
        catalog
    }
}

fn folder_name(folder: &Path) -> std::borrow::Cow<'_, str> {
    folder.file_name().unwrap_or_default().to_string_lossy()
}

struct LoadedExtension {
    manifests: manifest::Stamps,
    entry: String,
    /// Every JavaScript file the package was compiled from.
    files: Vec<FileStamp>,
    /// What the bytecode was compiled from, so a revision whose files are unchanged reuses it.
    content: blake3::Hash,
    /// The image file `dupin.icon` names, so a replaced or retargeted image loads again.
    icon: Option<IconStamp>,
    extension: Arc<Extension>,
}

struct IconStamp {
    /// As the manifest names it.
    name: String,
    stamp: FileStamp,
}

impl IconStamp {
    fn read(folder: &Path, name: &str) -> Result<Self, Diagnostic> {
        let unreadable = |diagnostic: Diagnostic| {
            diagnostic.file(manifest::PACKAGE_JSON).field("dupin.icon").hint(
                "Ship the image inside the package folder, or name an SF Symbol, such as `sf:puzzlepiece`.",
            )
        };
        let image = dupin_js_runtime::package_file(folder, name).map_err(unreadable)?;
        let stamp = FileStamp::read(image).map_err(|error| {
            unreadable(Diagnostic::new(
                "package-unreadable",
                format!("`{name}` can't be read: {error}"),
            ))
        })?;
        Ok(Self {
            name: name.to_owned(),
            stamp,
        })
    }

    fn is_current(&self, folder: &Path) -> bool {
        Self::read(folder, &self.name).is_ok_and(|now| now.stamp == self.stamp)
    }
}

#[derive(PartialEq, Eq)]
struct FileStamp {
    path: PathBuf,
    len: u64,
    modified: SystemTime,
}

impl FileStamp {
    fn read(path: PathBuf) -> std::io::Result<Self> {
        let metadata = path.metadata()?;
        Ok(Self {
            path,
            len: metadata.len(),
            modified: metadata.modified()?,
        })
    }
}

/// Reads the extension in `folder` as the scanner does, for `dupin ext` to check or run.
pub fn load_folder(folder: &Path) -> Result<Arc<Extension>, Diagnostic> {
    load(folder, None, false).map(|loaded| loaded.extension)
}

/// Loads the package in `folder` as one bundled with Dupin, without checking its seal.
#[cfg(test)]
pub(crate) fn load_bundled(folder: &Path) -> Result<Arc<Extension>, Diagnostic> {
    load(folder, None, true).map(|loaded| loaded.extension)
}

/// Loads the extension in `folder` as the scanner does and links it: what `dupin ext check`
/// reports, and what `dupin ext build` and `dupin ext dev` require before a result is kept.
pub fn check_folder(folder: &Path) -> Result<Arc<Extension>, Diagnostic> {
    let extension = load_folder(folder)?;
    extension
        .check(Instant::now() + QUERY_DEADLINE)
        .map_err(|error| match error {
            RunError::Failed(diagnostic) => diagnostic,
            RunError::Cancelled => Diagnostic::new("cancelled", "the check was cancelled"),
        })?;
    Ok(extension)
}

fn load(
    folder: &Path,
    previous: Option<LoadedExtension>,
    bundled: bool,
) -> Result<LoadedExtension, Diagnostic> {
    let trust = if bundled { Trust::Bundled } else { Trust::Own };
    load_with(folder, previous, bundled, |folder, entry, previous| {
        compile(folder, entry, previous, trust)
    })
}

/// `compile` reads the package's files only after they are stamped, so an edit that lands while
/// they are read leaves a stamp that no longer matches, and the next scan loads it again.
fn load_with(
    folder: &Path,
    previous: Option<LoadedExtension>,
    bundled: bool,
    compile: impl FnOnce(&Path, &str, Option<&LoadedExtension>) -> Result<(Compiled, blake3::Hash), Diagnostic>,
) -> Result<LoadedExtension, Diagnostic> {
    let manifests = manifest::Stamps::read(folder)?;
    let previous = match previous {
        Some(previous)
            if previous.manifests == manifests
                && revision(folder, &previous.entry).is_ok_and(|files| files == previous.files)
                && previous.icon.as_ref().is_none_or(|icon| icon.is_current(folder)) =>
        {
            return Ok(previous);
        }
        previous => previous,
    };

    let manifest = manifest::read(&manifests)?;
    let (icon, icon_stamp) = package_icon(folder, manifest.icon.as_ref())?;
    let files = revision(folder, &manifest.entry)?;
    let (compiled, content) = compile(folder, &manifest.entry, previous.as_ref())?;
    let package = package_id(&manifest, bundled)?;
    let revision = package_revision(folder, &content)?;
    Ok(LoadedExtension {
        manifests,
        entry: manifest.entry,
        files,
        content,
        icon: icon_stamp,
        extension: Arc::new(Extension {
            id: manifest.id,
            folder_name: folder_name(folder).into_owned(),
            name: manifest.name,
            version: manifest.version,
            keyword: manifest.keyword,
            root: folder.to_path_buf(),
            icon,
            default_search: manifest.default_search,
            empty_query: manifest.empty_query,
            matching: manifest.matching,
            preferences: manifest.preferences,
            exports: OnceLock::new(),
            catalogue: catalogue::Cached::new(manifest.matching),
            web_searches_noted: AtomicBool::new(false),
            waking: AtomicBool::new(false),
            package,
            revision,
            capabilities: manifest.capabilities,
            provides: manifest.provides,
            compiled,
        }),
    })
}

/// The icon `declared` names, and for an image file its stamp: the file must be inside the
/// package folder, as [`dupin_js_runtime::package_file`] checks, or the package fails to load,
/// as it would for a module.
fn package_icon(
    folder: &Path,
    declared: Option<&manifest::PackageIcon>,
) -> Result<(Option<dupin_core::Icon>, Option<IconStamp>), Diagnostic> {
    match declared {
        None => Ok((None, None)),
        Some(manifest::PackageIcon::Symbol(name)) => Ok((Some(dupin_core::Icon::Symbol(name.clone())), None)),
        Some(manifest::PackageIcon::File(name)) => {
            let stamp = IconStamp::read(folder, name)?;
            Ok((
                Some(dupin_core::Icon::Image(stamp.stamp.path.clone())),
                Some(stamp),
            ))
        }
    }
}

/// The revision grants are for: package.json and every module Dupin runs (`content`), so a
/// change to the declaration or the code is another revision and what Dupin never reads isn't.
/// Home Manager names the same revision in its declared grants, through `dupin ext revision`.
fn package_revision(folder: &Path, content: &blake3::Hash) -> Result<Revision, Diagnostic> {
    let package = std::fs::read(folder.join(manifest::PACKAGE_JSON)).map_err(|error| {
        Diagnostic::new("invalid-manifest", format!("Couldn't read it: {error}")).file(manifest::PACKAGE_JSON)
    })?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(&(package.len() as u64).to_le_bytes());
    hasher.update(&package);
    hasher.update(content.as_bytes());
    Revision::new(format!("blake3:{}", hasher.finalize().to_hex()))
        .map_err(|error| Diagnostic::new("host-error", error))
}

pub(crate) const BUNDLED_PREFIX: &str = "bundled:";

/// What the broker knows a package by: `name` in its package.json, whatever its folder is called.
/// A bundled package is `bundled:<name>`, which no package.json name can be, so grants and
/// choices never pass between it and a package in the extensions folder that reuses its name.
fn package_id(manifest: &manifest::Manifest, bundled: bool) -> Result<PackageId, Diagnostic> {
    let prefix = if bundled { BUNDLED_PREFIX } else { "" };
    PackageId::new(format!("{prefix}{}", manifest.id)).map_err(|error| {
        Diagnostic::new("invalid-manifest", error)
            .file(manifest::PACKAGE_JSON)
            .field("name")
    })
}

/// What the broker knows the package in `folder` by, the revision Dupin gives it, which
/// declared grants name, and what it declares.
pub fn declaration_of(folder: &Path) -> Result<(PackageId, Revision, Capabilities), Diagnostic> {
    let manifests = manifest::Stamps::read(folder)?;
    let manifest = manifest::read(&manifests)?;
    let sources = Sources::read(folder, &manifest.entry)?;
    Ok((
        package_id(&manifest, false)?,
        package_revision(folder, &content_hash(&sources))?,
        manifest.capabilities,
    ))
}

fn compile(
    folder: &Path,
    entry: &str,
    previous: Option<&LoadedExtension>,
    trust: Trust,
) -> Result<(Compiled, blake3::Hash), Diagnostic> {
    let sources = Sources::read(folder, entry)?;
    let content = content_hash(&sources);
    if let Some(previous) = previous.filter(|previous| previous.content == content) {
        return Ok((previous.extension.compiled.clone(), content));
    }
    let compiled = host()
        .compile(&sources, trust, QUERY_DEADLINE, &|| false)
        .map_err(|error| match error {
            RunError::Failed(diagnostic) => diagnostic,
            RunError::Cancelled => {
                Diagnostic::new("host-error", "the extension host cancelled compiling the package")
                    .file(sources.entry())
            }
        })?;
    Ok((compiled, content))
}

/// Names and texts with their lengths, so moving text between files changes the hash.
fn content_hash(sources: &Sources) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new();
    let parts =
        std::iter::once(sources.entry()).chain(sources.files().flat_map(|(name, source)| [name, source]));
    for part in parts {
        hasher.update(&(part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    hasher.finalize()
}

/// The files the package with `entry` would be compiled from now, to compare with a loaded revision.
fn revision(folder: &Path, entry: &str) -> Result<Vec<FileStamp>, Diagnostic> {
    dupin_js_runtime::source_files(folder, entry)?
        .into_iter()
        .map(|(_, path)| {
            FileStamp::read(path.clone()).map_err(|error| {
                Diagnostic::new("package-unreadable", format!("Couldn't read it: {error}"))
                    .file(path.display().to_string())
            })
        })
        .collect()
}

fn read_capped(path: &Path, cap: usize) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take(cap as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > cap {
        return Err(format!("larger than {} KiB", cap / 1024));
    }
    Ok(bytes)
}

/// Scans now and after changes under `directory`, delivering changed catalogues in order. The
/// user's folder holds packages the installed extensions list pins; the bundled folder only the
/// packages this build sealed.
pub fn watch(
    directory: PathBuf,
    origin: Origin,
    deliver: impl Fn(Catalog) + Send + 'static,
) -> std::io::Result<Watch> {
    let integrity: Option<Integrity> = match origin {
        Origin::User => installed::shared().map(|installed| {
            let bundled = bundled::directory()
                .map(|directory| bundled::packages(&directory))
                .unwrap_or_default();
            let names = bundled.into_iter().map(|(_, name, _)| name).collect();
            let mut integrity = dupin_extension_install::Integrity::new(Box::new(installed), names);
            Box::new(move |folder: &Path| integrity.check(folder)) as _
        }),
        Origin::Bundled => {
            let seals = bundled::Seals::shipped();
            Some(Box::new(move |folder: &Path| seals.check(folder)))
        }
    };
    let mut scanner = Scanner {
        integrity,
        bundled: origin == Origin::Bundled,
        ..Scanner::default()
    };
    // `dupin ext render` starts typing a fixed moment after the launcher starts, which a first
    // scan compiling every package on a worker can outlast; scanned here, before the first
    // frame, the packages are registered by then, and the worker's first scan finds them
    // unchanged.
    if fixtures::headless().is_some()
        && let Some(catalog) = scanner.update(&directory, None)
    {
        deliver(catalog);
    }
    watch_with(directory, scanner, deliver)
}

fn watch_with(
    directory: PathBuf,
    mut scanner: Scanner,
    deliver: impl Fn(Catalog) + Send + 'static,
) -> std::io::Result<Watch> {
    Watch::start(
        directory,
        crate::catalog_watch::Depth::Trees,
        move |directory, problem, files| {
            let catalog = scanner.update(directory, problem);
            files.extend(scanner.files());
            catalog
        },
        deliver,
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::manifest::{PACKAGE_JSON, WASM_MANIFEST};
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    pub(super) const EXTENSION_HOST_ENV: &str = "DUPIN_EXTENSION_HOST_TEST";

    #[test]
    fn a_package_icon_file_must_be_a_file_inside_the_package() {
        use manifest::PackageIcon;
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.png"), b"png").unwrap();
        let package = tempfile::tempdir().unwrap();
        let root = package.path();
        fs::create_dir_all(root.join("assets/folder.png")).unwrap();
        fs::write(root.join("assets/icon.png"), b"png").unwrap();
        symlink(root.join("assets/icon.png"), root.join("inside.png")).unwrap();
        symlink(outside.path().join("secret.png"), root.join("escape.png")).unwrap();
        symlink(outside.path(), root.join("linked")).unwrap();
        let file =
            |name: &str| package_icon(root, Some(&PackageIcon::File(name.into()))).map(|(icon, _)| icon);

        let canonical = root.canonicalize().unwrap().join("assets/icon.png");
        assert_eq!(
            file("assets/icon.png").unwrap(),
            Some(dupin_core::Icon::Image(canonical.clone()))
        );
        assert_eq!(
            file("inside.png").unwrap(),
            Some(dupin_core::Icon::Image(canonical))
        );
        assert_eq!(
            package_icon(root, Some(&PackageIcon::Symbol("star".into())))
                .unwrap()
                .0,
            Some(dupin_core::Icon::Symbol("star".into()))
        );
        assert!(package_icon(root, None).unwrap().0.is_none());
        for (name, code) in [
            ("escape.png", "symlink-escape"),
            ("linked/secret.png", "symlink-escape"),
            ("missing.png", "package-unreadable"),
            ("assets/folder.png", "package-unreadable"),
        ] {
            let diagnostic = file(name).unwrap_err();
            assert_eq!(
                (diagnostic.code.as_ref(), diagnostic.field.as_deref()),
                (code, Some("dupin.icon")),
                "{name}: {diagnostic}"
            );
        }
    }

    /// Not a test: [`helper_command`] starts this test binary, running only this function, as
    /// the extension host.
    #[test]
    fn extension_host() {
        if std::env::var_os(EXTENSION_HOST_ENV).is_some() {
            dupin_extension_host::serve();
        }
    }

    /// An extension that routes and lists like any other but has no code to run.
    pub(crate) fn extension(name: &str, keyword: &str) -> Extension {
        let id = name.to_lowercase().replace(' ', "-");
        Extension {
            folder_name: format!("{id}-folder"),
            package: PackageId::new(id.clone()).unwrap(),
            id,
            name: name.into(),
            version: "1.0.0".into(),
            keyword: keyword.into(),
            root: PathBuf::from("/fixture"),
            icon: None,
            default_search: false,
            empty_query: false,
            matching: Default::default(),
            preferences: Default::default(),
            exports: OnceLock::new(),
            catalogue: catalogue::Cached::default(),
            web_searches_noted: AtomicBool::new(false),
            waking: AtomicBool::new(false),
            revision: Revision::new("blake3:fixture").unwrap(),
            capabilities: Capabilities::default(),
            provides: Default::default(),
            compiled: Compiled::empty(),
        }
    }

    fn folder() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("dupin-extensions-")
            .tempdir()
            .unwrap()
    }

    const SEARCH: &str = "export const search = () => [];";

    fn install(root: &Path, folder: &str, package: &str, main: Option<&str>) {
        let dir = root.join(folder);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(PACKAGE_JSON), package).unwrap();
        if let Some(main) = main {
            fs::write(dir.join("main.js"), main).unwrap();
        }
    }

    fn package_json(id: &str, keyword: &str, main: Option<&str>) -> String {
        let main = main
            .map(|main| format!(r#", "main": "{main}""#))
            .unwrap_or_default();
        format!(
            r#"{{"name": "{id}", "version": "0.1.0", "dupin": {{"title": "Word Count", "keyword": "{keyword}"{main}}}}}"#
        )
    }

    fn install_javascript(root: &Path, folder: &str, id: &str, keyword: &str, files: &[(&str, &str)]) {
        let dir = root.join(folder);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(PACKAGE_JSON), package_json(id, keyword, None)).unwrap();
        for (name, source) in files {
            let path = dir.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, source).unwrap();
        }
    }

    fn observe(directory: &Path) -> (Watch, mpsc::Receiver<Catalog>) {
        let (updates, received) = mpsc::channel();
        let watcher = watch_with(directory.to_owned(), Scanner::default(), move |catalog| {
            let _ = updates.send(catalog);
        })
        .unwrap();
        (watcher, received)
    }

    fn wait_for_keyword(received: &mpsc::Receiver<Catalog>, keyword: Option<&str>) {
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            let catalog = received
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| panic!("no catalogue with keyword {keyword:?}: {error}"));
            let keywords = catalog
                .extensions
                .iter()
                .map(|extension| extension.keyword.as_str());
            if keywords.eq(keyword) {
                assert!(catalog.problems.is_empty(), "{:?}", catalog.problems);
                return;
            }
        }
    }

    #[test]
    fn retargeted_catalogue_symlink_is_rewatched_before_later_edits() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let fixture = folder();
        let root = fixture.path();
        let directory = root.join("extensions");
        let first = root.join("generation-one");
        let second = root.join("generation-two");
        install(
            &first,
            "words",
            &package_json("words", "first", None),
            Some(SEARCH),
        );
        install(
            &second,
            "words",
            &package_json("words", "second", None),
            Some(SEARCH),
        );
        symlink(&first, &directory).unwrap();

        let (watcher, received) = observe(&directory);
        wait_for_keyword(&received, Some("first"));

        let replacement = root.join("replacement");
        symlink(&second, &replacement).unwrap();
        fs::rename(&replacement, &directory).unwrap();
        wait_for_keyword(&received, Some("second"));

        fs::write(
            second.join("words").join(PACKAGE_JSON),
            package_json("words", "edited", None),
        )
        .unwrap();
        wait_for_keyword(&received, Some("edited"));
        drop(watcher);
    }

    #[test]
    fn retargeted_extension_symlink_hop_is_rewatched_before_later_edits() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let fixture = folder();
        let root = fixture.path();
        let directory = root.join("extensions");
        let links = root.join("links");
        fs::create_dir(&directory).unwrap();
        fs::create_dir(&links).unwrap();
        install(root, "first", &package_json("words", "first", None), Some(SEARCH));
        install(
            root,
            "second",
            &package_json("words", "second", None),
            Some(SEARCH),
        );
        symlink("../first", links.join("current")).unwrap();
        symlink("../links/current", directory.join("words")).unwrap();

        let (watcher, received) = observe(&directory);
        wait_for_keyword(&received, Some("first"));

        let replacement = links.join("replacement");
        symlink("../second", &replacement).unwrap();
        fs::rename(&replacement, links.join("current")).unwrap();
        wait_for_keyword(&received, Some("second"));

        fs::write(
            root.join("second").join(PACKAGE_JSON),
            package_json("words", "edited", None),
        )
        .unwrap();
        wait_for_keyword(&received, Some("edited"));
        drop(watcher);
    }

    #[test]
    fn moved_catalogue_directory_is_rewatched_before_later_edits() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let fixture = folder();
        let root = fixture.path();
        let directory = root.join("extensions");
        install(
            &directory,
            "words",
            &package_json("words", "first", None),
            Some(SEARCH),
        );

        let (watcher, received) = observe(&directory);
        wait_for_keyword(&received, Some("first"));

        fs::rename(&directory, root.join("moved")).unwrap();
        wait_for_keyword(&received, None);

        install(
            &directory,
            "words",
            &package_json("words", "replacement", None),
            Some(SEARCH),
        );
        wait_for_keyword(&received, Some("replacement"));

        fs::write(
            directory.join("words").join(PACKAGE_JSON),
            package_json("words", "edited", None),
        )
        .unwrap();
        wait_for_keyword(&received, Some("edited"));
        drop(watcher);
    }

    #[test]
    fn a_scan_names_the_files_of_packages_it_loaded_and_of_those_it_could_not() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let fixture = folder();
        let directory = fixture.path().join("extensions");
        install_javascript(
            &directory,
            "words",
            "words",
            "words",
            &[
                ("main.js", "export { search } from './lib/search.js';"),
                ("lib/search.js", SEARCH),
            ],
        );
        install_javascript(
            &directory,
            "broken",
            "broken",
            "broken",
            &[("main.js", "export const = ;")],
        );
        let mut scanner = Scanner::default();
        let catalog = scanner.scan(&directory);
        assert_eq!(catalog.extensions.len(), 1, "{:?}", catalog.problems);

        let canonical = directory.canonicalize().unwrap();
        let mut files = scanner.files();
        files.sort();
        let mut expected = vec![
            directory.join("broken").join(PACKAGE_JSON),
            canonical.join("broken/main.js"),
            directory.join("words").join(PACKAGE_JSON),
            canonical.join("words/lib/search.js"),
            canonical.join("words/main.js"),
        ];
        expected.sort();
        assert_eq!(files, expected);
    }

    #[test]
    fn a_missing_folder_is_an_empty_catalogue() {
        let catalog = Scanner::default().scan(&folder().path().join("absent"));
        assert!(catalog.extensions.is_empty());
        assert!(catalog.problems.is_empty());
    }

    #[test]
    fn catalogue_updates_reuse_unchanged_extensions_and_skip_unchanged_deliveries() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let fixture = folder();
        let root = fixture.path();
        let directory = root.join("extensions");
        install(
            &directory,
            "one",
            &package_json("one", "first", None),
            Some(SEARCH),
        );
        install(
            &directory,
            "two",
            &package_json("two", "second", None),
            Some(SEARCH),
        );
        let mut scanner = Scanner::default();
        let initial = scanner.update(&directory, None).unwrap();

        fs::write(root.join("config.toml"), "extensions = true").unwrap();
        assert!(scanner.update(&directory, None).is_none());

        fs::write(
            directory.join("one/main.js"),
            "export const search = () => [{ title: 'changed' }];",
        )
        .unwrap();
        let updated = scanner.update(&directory, None).unwrap();
        assert!(!Arc::ptr_eq(&initial.extensions[0], &updated.extensions[0]));
        assert!(Arc::ptr_eq(&initial.extensions[1], &updated.extensions[1]));

        fs::write(
            directory.join("two").join(PACKAGE_JSON),
            package_json("two", "reloaded", None),
        )
        .unwrap();
        let reloaded = scanner.update(&directory, None).unwrap();
        assert!(Arc::ptr_eq(&updated.extensions[0], &reloaded.extensions[0]));
        assert!(!Arc::ptr_eq(&updated.extensions[1], &reloaded.extensions[1]));
        assert_eq!(reloaded.extensions[1].keyword, "reloaded");
        assert!(scanner.update(&directory, None).is_none());

        fs::remove_file(directory.join("one/main.js")).unwrap();
        let removed = scanner.update(&directory, None).unwrap();
        assert_eq!(removed.extensions.len(), 1);
        assert!(Arc::ptr_eq(&reloaded.extensions[1], &removed.extensions[0]));
        assert!(removed.problems[0].starts_with("one:"));
        assert!(scanner.update(&directory, None).is_none());
    }

    #[test]
    fn valid_extensions_load_and_invalid_ones_are_reported_and_skipped() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let fixture = folder();
        let root = fixture.path();
        install(root, "a-valid", &package_json("words", "wc", None), Some(SEARCH));
        install(
            root,
            "b-duplicate",
            &package_json("words", "wc2", None),
            Some(SEARCH),
        );
        install(root, "c-bad-id", &package_json("Words!", "w", None), Some(SEARCH));
        install(
            root,
            "d-spaced-keyword",
            &package_json("spaced", "w c", None),
            Some(SEARCH),
        );
        install(root, "e-missing-entry", &package_json("missing", "m", None), None);
        install(
            root,
            "f-unknown-key",
            &package_json("x", "x", None).replace(r#""keyword""#, r#""extra": 1, "keyword""#),
            Some(SEARCH),
        );
        install(
            root,
            "g-escape",
            &package_json("escape", "e", Some("../a-valid/main.js")),
            None,
        );
        install(
            root,
            "h-broken",
            &package_json("broken", "b", None),
            Some("export function search( {"),
        );
        fs::create_dir_all(root.join("i-wasm")).unwrap();
        fs::write(
            root.join("i-wasm").join(WASM_MANIFEST),
            "id = \"wasm\"\nname = \"Word Count\"\nversion = \"0.1.0\"\nkeyword = \"wa\"\nentry = \"guest.wasm\"\n",
        )
        .unwrap();
        fs::write(root.join("i-wasm/guest.wasm"), b"\0asm\x01\0\0\0").unwrap();
        fs::create_dir_all(root.join("j-no-manifest")).unwrap();
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::write(root.join("k-outside.js"), SEARCH).unwrap();
        install(root, "k-symlink", &package_json("symlink", "s", None), None);
        symlink(root.join("k-outside.js"), root.join("k-symlink/main.js")).unwrap();

        let catalog = Scanner::default().scan(root);
        let ids: Vec<_> = catalog.extensions.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["words"]);
        let words = &catalog.extensions[0];
        assert_eq!(
            (words.name.as_str(), words.keyword.as_str()),
            ("Word Count", "wc")
        );
        let problems = catalog.problems.join("\n");
        for (folder, reason) in [
            ("b-duplicate", "used by another extension"),
            ("c-bad-id", "`name` must be"),
            ("d-spaced-keyword", "`dupin.keyword` must be"),
            ("e-missing-entry", "is not a module in the package folder"),
            ("f-unknown-key", "`dupin.extra` is not a key Dupin reads"),
            ("g-escape", "`dupin.main` must be a relative path"),
            ("h-broken", "main.js:1:"),
            (
                "i-wasm",
                "extension.toml: Wasm extensions were removed; port to JavaScript",
            ),
            ("j-no-manifest", "package.json"),
            ("k-symlink", "main.js"),
        ] {
            assert!(
                problems
                    .lines()
                    .any(|line| line.starts_with(folder) && line.contains(reason)),
                "{folder}: {reason}\n{problems}"
            );
        }
        assert_eq!(catalog.problems.len(), 10, "{problems}");
    }

    #[test]
    fn a_folder_that_no_longer_matches_the_lock_stays_off() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let fixture = folder();
        let root = fixture.path().join("extensions");
        install(&root, "words", &package_json("words", "wc", None), Some(SEARCH));
        let key = dupin_storage::DataKey::generate().unwrap();
        let storage = |key: &dupin_storage::DataKey| {
            Some(Arc::new(dupin_storage::Storage::new(
                fixture.path().join("storage"),
                dupin_storage::MemoryKey::new(key.clone()),
            )))
        };
        let installed = Arc::new(installed::Installed::new(storage(&key)));
        let mut lock = dupin_extension_install::Lock::new();
        lock.packages.insert(
            "words".into(),
            dupin_extension_install::Entry {
                source: "github:someone/words".into(),
                rev: None,
                commit: None,
                content_hash: dupin_extension_install::content_hash(&root.join("words")).unwrap(),
                installed_at: "2026-01-01T00:00:00Z".into(),
                version: "0.1.0".into(),
                keyword: "wc".into(),
                requested: vec![],
            },
        );
        installed.write(&lock).unwrap();
        let mut integrity =
            dupin_extension_install::Integrity::new(Box::new(installed.clone()), Default::default());
        let mut scanner = Scanner {
            integrity: Some(Box::new(move |folder: &Path| integrity.check(folder))),
            ..Scanner::default()
        };
        assert_eq!(scanner.scan(&root).extensions.len(), 1);

        fs::write(
            root.join("words/main.js"),
            "export const search = () => [{ title: 'tampered' }];",
        )
        .unwrap();
        let catalog = scanner.scan(&root);
        assert!(catalog.extensions.is_empty());
        assert!(
            catalog.problems[0].starts_with("words: disabled")
                && catalog.problems[0].contains("what `dupin ext install` placed"),
            "{:?}",
            catalog.problems
        );

        fs::write(root.join("words/main.js"), SEARCH).unwrap();
        assert_eq!(scanner.scan(&root).extensions.len(), 1);
        drop(scanner);
        drop(installed);
        let other = dupin_storage::DataKey::generate().unwrap();
        let wrong = installed::Installed::new(storage(&other));
        let mut integrity =
            dupin_extension_install::Integrity::new(Box::new(Arc::new(wrong)), Default::default());
        let mut scanner = Scanner {
            integrity: Some(Box::new(move |folder: &Path| integrity.check(folder))),
            ..Scanner::default()
        };
        let catalog = scanner.scan(&root);
        assert!(
            catalog.extensions.is_empty(),
            "a list sealed with another key keeps every folder off"
        );
        assert!(
            catalog.problems[0].contains("couldn't be read"),
            "{:?}",
            catalog.problems
        );
    }

    #[test]
    fn the_bundled_folder_loads_only_what_the_build_sealed() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let fixture = folder();
        let root = fixture.path();
        install(root, "words", &package_json("words", "wc", None), Some(SEARCH));
        let seals = bundled::Seals::of(&[&root.join("words")]);
        install(root, "added", &package_json("added", "add", None), Some(SEARCH));
        let mut scanner = Scanner {
            integrity: Some(Box::new(move |folder: &Path| seals.check(folder))),
            ..Scanner::default()
        };
        let catalog = scanner.scan(root);
        assert_eq!(catalog.extensions.len(), 1);
        assert_eq!(catalog.extensions[0].id, "words");
        assert!(
            catalog.problems[0].starts_with("added: isn't"),
            "{:?}",
            catalog.problems
        );

        fs::write(
            root.join("words/main.js"),
            "export const search = () => [{ title: 'tampered' }];",
        )
        .unwrap();
        assert!(scanner.scan(root).extensions.is_empty());
    }

    #[test]
    fn a_revision_covers_the_manifest_and_the_code() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let fixture = folder();
        let words = fixture.path().join("words");
        let revision = || load_folder(&words).unwrap().revision.clone();
        install(
            fixture.path(),
            "words",
            &package_json("words", "wc", None),
            Some(SEARCH),
        );
        let first = revision();
        assert_eq!(revision(), first);

        fs::write(words.join(PACKAGE_JSON), package_json("words", "words", None)).unwrap();
        assert_ne!(revision(), first);
        fs::write(words.join(PACKAGE_JSON), package_json("words", "wc", None)).unwrap();
        assert_eq!(revision(), first);
        fs::write(
            words.join("main.js"),
            "export const search = () => [{ title: 'x' }];",
        )
        .unwrap();
        assert_ne!(revision(), first);
    }

    #[test]
    fn oversized_manifests_are_rejected_before_parsing() {
        let fixture = folder();
        let root = fixture.path();
        install(root, "big", &" ".repeat(64 * 1024 + 1), None);
        let catalog = Scanner::default().scan(root);
        assert!(
            catalog.problems[0].contains("larger than 64 KiB"),
            "{:?}",
            catalog.problems
        );
    }

    #[test]
    fn packages_load_with_their_imports_and_entries() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let fixture = folder();
        let root = fixture.path();
        install_javascript(
            root,
            "b-js",
            "js",
            "js",
            &[
                (
                    "main.js",
                    "import { title } from './lib/title.js'; export const search = () => [{ title }];",
                ),
                ("lib/title.js", "export const title = 'first';"),
            ],
        );
        install_javascript(
            root,
            "c-entry",
            "entry",
            "en",
            &[("src/index.mjs", "export function search() { return []; }")],
        );
        fs::write(
            root.join("c-entry").join(PACKAGE_JSON),
            package_json("entry", "en", Some("./src/./index.mjs")),
        )
        .unwrap();
        install_javascript(
            root,
            "d-broken",
            "broken",
            "br",
            &[("main.js", "export function search( {")],
        );
        install_javascript(root, "e-no-main", "nomain", "nm", &[("index.js", "")]);

        let mut scanner = Scanner::default();
        let catalog = scanner.update(root, None).unwrap();
        let loaded: Vec<_> = catalog
            .extensions
            .iter()
            .map(|extension| extension.id.as_str())
            .collect();
        assert_eq!(loaded, ["js", "entry"]);
        let problems = catalog.problems.join("\n");
        assert!(problems.contains("d-broken: main.js:1:"), "{problems}");
        assert!(
            problems.contains("e-no-main: main.js: is not a module in the package folder"),
            "{problems}"
        );
        assert_eq!(catalog.problems.len(), 2, "{problems}");
        assert!(scanner.update(root, None).is_none());

        // An edit to an imported file, not only to the entry, is a new revision.
        fs::write(
            root.join("b-js/lib/title.js"),
            "export const title = 'second edit';",
        )
        .unwrap();
        let updated = scanner.update(root, None).unwrap();
        assert!(!Arc::ptr_eq(&catalog.extensions[0], &updated.extensions[0]));
        assert!(Arc::ptr_eq(&catalog.extensions[1], &updated.extensions[1]));
        let rows = updated.extensions[0]
            .query(
                "",
                10,
                Instant::now() + QUERY_DEADLINE,
                || false,
                &(Arc::new(dupin_extension_host::NoServices) as _),
            )
            .unwrap();
        assert_eq!(rows[0].title, "second edit");

        fs::write(root.join("b-js/extra.js"), "export {};").unwrap();
        assert!(scanner.update(root, None).is_some());
    }

    #[test]
    fn a_ported_package_loads_beside_its_leftover_wasm_manifest() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let fixture = folder();
        let root = fixture.path();
        install_javascript(root, "js", "js", "js", &[("main.js", SEARCH)]);
        fs::write(root.join("js").join(WASM_MANIFEST), "entry = \"guest.wasm\"\n").unwrap();
        let mut scanner = Scanner::default();
        let catalog = scanner.update(root, None).unwrap();
        assert_eq!(catalog.extensions.len(), 1, "{:?}", catalog.problems);

        fs::write(
            root.join("js").join(PACKAGE_JSON),
            r#"{"name": "js", "version": "0.1.0"}"#,
        )
        .unwrap();
        let catalog = scanner.update(root, None).unwrap();
        assert!(catalog.extensions.is_empty());
        assert_eq!(
            catalog.problems,
            ["js: extension.toml: Wasm extensions were removed; port to JavaScript"]
        );
    }

    #[test]
    fn an_edit_while_a_package_is_read_is_picked_up_by_the_next_scan() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let fixture = folder();
        let root = fixture.path();
        install_javascript(
            root,
            "js",
            "js",
            "js",
            &[("main.js", "export const search = () => [{ title: 'old' }];")],
        );
        let folder = root.join("js");
        let edited = folder.join("main.js");
        let loaded = load_with(&folder, None, false, |folder, entry, previous| {
            let compiled = compile(folder, entry, previous, Trust::Own);
            fs::write(&edited, "export const search = () => [{ title: 'new edit' }];").unwrap();
            compiled
        })
        .unwrap();
        let before = Arc::clone(&loaded.extension);
        let reloaded = load(&folder, Some(loaded), false).unwrap();
        assert!(!Arc::ptr_eq(&before, &reloaded.extension));
        let current = Arc::clone(&reloaded.extension);
        let unchanged = load(&folder, Some(reloaded), false).unwrap();
        assert!(Arc::ptr_eq(&current, &unchanged.extension));
    }

    #[test]
    fn a_retargeted_or_replaced_package_icon_loads_the_package_again() {
        dupin_extension_host::skip_inside_a_sandbox!();
        let fixture = folder();
        let folder = fixture.path().join("marks");
        fs::create_dir_all(folder.join("assets")).unwrap();
        fs::write(
            folder.join(PACKAGE_JSON),
            r#"{"name": "marks", "version": "0.1.0", "dupin": {"title": "Marks", "keyword": "bm", "icon": "icon.png"}}"#,
        )
        .unwrap();
        fs::write(folder.join("main.js"), SEARCH).unwrap();
        fs::write(folder.join("assets/a.png"), b"a").unwrap();
        fs::write(folder.join("assets/b.png"), b"bb").unwrap();
        symlink("assets/a.png", folder.join("icon.png")).unwrap();
        let image = |name: &str| dupin_core::Icon::Image(folder.canonicalize().unwrap().join(name));

        let loaded = load(&folder, None, false).unwrap();
        assert_eq!(loaded.extension.icon, Some(image("assets/a.png")));
        let first = Arc::clone(&loaded.extension);
        let unchanged = load(&folder, Some(loaded), false).unwrap();
        assert!(Arc::ptr_eq(&first, &unchanged.extension));

        fs::remove_file(folder.join("icon.png")).unwrap();
        symlink("assets/b.png", folder.join("icon.png")).unwrap();
        let retargeted = load(&folder, Some(unchanged), false).unwrap();
        assert_eq!(retargeted.extension.icon, Some(image("assets/b.png")));

        fs::remove_file(folder.join("icon.png")).unwrap();
        let Err(gone) = load(&folder, Some(retargeted), false) else {
            panic!("a missing icon fails the package");
        };
        assert_eq!(
            (gone.code.as_ref(), gone.field.as_deref()),
            ("package-unreadable", Some("dupin.icon"))
        );
    }
}
