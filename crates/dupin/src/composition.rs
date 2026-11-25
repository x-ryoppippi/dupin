//! What `[advanced]` composes over the feature manifest: each native source's filter stages,
//! the actions added to the kinds and the sources the main search appends. Every source and
//! row is built from one composition, so a reload reaches them all at once.
//!
//! A package's filter composed onto a source runs on bounded batches of that source's titles,
//! and only while the broker lets the package see them: Dupin's own source keeps answering
//! whenever the package is missing, unconsented, slow or wrong.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};
use std::time::{Duration, Instant};

use dupin_broker::{RowField, RowKind};
use dupin_core::config::{Advanced, contribution};
use dupin_core::filter::{BatchStage, Filters, Hit, Sorter, SourceFilters, Stages};
use dupin_core::kind::{Kinds, Subject};
use dupin_extension_bridge::{Bridge, Request, Trigger};
use dupin_js_runtime::{FilterInput, FilterStage};
use dupin_mac::kinds::Rows;

use crate::extensions::Extension;
use crate::features::{self, Source};

/// How long a package's filter may take on one batch. It runs as the user types, so past
/// this the source shows its own matches instead.
const FILTER_DEADLINE: Duration = Duration::from_millis(250);

pub struct Composition {
    kinds: Kinds,
    filters: Filters,
    /// The stages of each source that matches its own candidates.
    sources: BTreeMap<&'static str, SourceFilters>,
    guests: Arc<Guests>,
}

impl Composition {
    /// The feature manifest's composition, as with `[advanced]` absent.
    pub fn native(kinds: Kinds) -> Self {
        let filters = Filters::native();
        let sources = features::sources()
            .filter_map(|source| Some((source.name, SourceFilters::new(default_stages(&filters, source)?))))
            .collect();
        Self {
            kinds,
            filters,
            sources,
            guests: Arc::default(),
        }
    }

    pub fn kinds(&self) -> &Kinds {
        &self.kinds
    }

    /// Where the packages a composed filter runs in are handed over.
    pub fn guests(&self) -> &Arc<Guests> {
        &self.guests
    }

    /// What the source `name`, which matches its own candidates, builds its rows with.
    pub fn rows(&self, name: &str) -> Rows {
        let filters = self
            .sources
            .get(name)
            .unwrap_or_else(|| panic!("the source {name} has no matcher Dupin registers"))
            .clone();
        Rows {
            filters,
            kinds: self.kinds.clone(),
        }
    }

    /// Compose `advanced`, which [`check`] accepted, over the manifest's defaults: a source or
    /// kind it no longer names goes back to them.
    pub fn apply(&self, advanced: &Advanced) -> Recomposed {
        let kinds = self.kinds.configure(&advanced.kinds);
        let mut sources = false;
        for (name, filters) in &self.sources {
            let source = features::sources()
                .find(|source| source.name == *name)
                .expect("each composed source is registered");
            let Some(mut stages) = default_stages(&self.filters, source) else {
                continue;
            };
            if let Some(configured) = advanced.sources.get(*name) {
                let guest = |reference: &str, stage| {
                    let (package, filter) = contribution(reference)?;
                    Some(Arc::new(GuestFilter {
                        reference: reference.to_owned(),
                        package: package.to_owned(),
                        filter: filter.to_owned(),
                        stage,
                        rows: source.rows?,
                        guests: self.guests.clone(),
                    }) as Arc<dyn BatchStage>)
                };
                if let Some(matcher) = configured.matcher.as_deref() {
                    match self.filters.matcher(matcher) {
                        Some(native) => stages.matcher = native,
                        None => stages.batch_matcher = guest(matcher, FilterStage::Matcher),
                    }
                }
                if let Some(sorter) = configured.sorter.as_deref() {
                    match Sorter::named(sorter) {
                        Some(native) => stages.sorter = native,
                        None => stages.batch_sorter = guest(sorter, FilterStage::Sorter),
                    }
                }
            }
            sources |= filters.set(stages);
        }
        Recomposed { sources, kinds }
    }
}

/// What applying a composition changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recomposed {
    /// A source's filter stages.
    pub sources: bool,
    /// The actions configured for a kind, which rows already built lack.
    pub kinds: bool,
}

impl Recomposed {
    pub fn any(self) -> bool {
        self.sources || self.kinds
    }
}

fn default_stages(filters: &Filters, source: &Source) -> Option<Stages> {
    Some(Stages::native(
        filters.matcher(source.matcher?)?,
        Sorter::Relevance,
    ))
}

/// Whether `advanced` names only what Dupin has, and composes each source with stages it
/// supports, so that applying it can't silently fall back.
pub fn check(advanced: &Advanced) -> Result<(), String> {
    let filters = Filters::native();
    let known = || {
        features::sources()
            .map(|source| source.name)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let source = |table: &str, name: &str| {
        features::sources()
            .find(|source| source.name == name)
            .ok_or_else(|| {
                format!(
                    "{table} names `{name}`, which isn't a source Dupin has ({})",
                    known()
                )
            })
    };
    for (name, stages) in &advanced.sources {
        let table = format!("[advanced.sources.{name}]");
        let found = source(&table, name)?;
        if found.matcher.is_none() {
            return Err(format!(
                "{table}: `{name}` matches through an index of its own, so it takes no matcher or sorter"
            ));
        }
        if let Some(matcher) = &stages.matcher
            && filters.matcher(matcher).is_none()
            && contribution(matcher).is_none()
        {
            return Err(format!(
                "{table}: `{matcher}` isn't a matcher Dupin has ({}) or a package's, written `<package>/<filter>`",
                filters.matcher_names().collect::<Vec<_>>().join(", ")
            ));
        }
        if let Some(sorter) = &stages.sorter
            && Sorter::named(sorter).is_none()
            && contribution(sorter).is_none()
        {
            return Err(format!(
                "{table}: `{sorter}` isn't a sorter Dupin has ({}) or a package's, written `<package>/<filter>`",
                Sorter::ALL.map(Sorter::name).join(", ")
            ));
        }
    }
    let route = "advanced.commands.default.append_sources";
    for name in &advanced.commands.default.append_sources {
        let found = source(route, name)?;
        if found.command == crate::features::apps::APPS {
            return Err(format!(
                "{route} names `{name}`, which is the main search's own source"
            ));
        }
        if found.command == crate::features::files::BROWSE {
            return Err(format!(
                "{route} names `{name}`, which lists only a folder being browsed"
            ));
        }
    }
    Ok(())
}

/// The packages composed filters run in and the broker that decides what they see, which the
/// app hands over once it has them.
#[derive(Default)]
pub struct Guests {
    bridge: OnceLock<Bridge>,
    extensions: RwLock<Vec<Arc<Extension>>>,
    /// Each problem is logged once, so a failing filter doesn't log on every key.
    logged: Mutex<HashSet<String>>,
}

impl Guests {
    pub fn set_bridge(&self, bridge: Bridge) {
        let _ = self.bridge.set(bridge);
    }

    /// The packages that run now.
    pub fn set_extensions(&self, extensions: Vec<Arc<Extension>>) {
        *self.extensions.write().unwrap_or_else(PoisonError::into_inner) = extensions;
    }

    fn extension(&self, package: &str) -> Option<Arc<Extension>> {
        self.extensions
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|extension| extension.id == package)
            .cloned()
    }

    fn note(&self, problem: String) {
        if self
            .logged
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(problem.clone())
        {
            crate::log::warn(&problem);
        }
    }
}

/// A package's `filters[name]`, composed onto a source whose rows are `rows`.
struct GuestFilter {
    /// `<package>/<filter>`, as config.toml names it.
    reference: String,
    package: String,
    filter: String,
    stage: FilterStage,
    rows: RowKind,
    guests: Arc<Guests>,
}

impl GuestFilter {
    /// What the broker must let the package see first: the titles of the source's rows, and
    /// for a matcher what was typed.
    fn requests(&self) -> Vec<Request> {
        let mut requests = vec![Request::Rows {
            kind: self.rows,
            field: RowField::Title,
        }];
        if self.stage == FilterStage::Matcher {
            requests.push(Request::Query);
        }
        requests
    }
}

impl BatchStage for GuestFilter {
    fn name(&self) -> &str {
        &self.reference
    }

    fn run(&self, query: &str, titles: &[&str]) -> Option<Vec<Hit>> {
        let reference = &self.reference;
        if titles.len() > dupin_js_runtime::filter::MAX_CANDIDATES {
            self.guests.note(format!(
                "{reference} wasn't run: the source has more than {} candidates, more than a filter takes",
                dupin_js_runtime::filter::MAX_CANDIDATES
            ));
            return None;
        }
        let Some(extension) = self.guests.extension(&self.package) else {
            self.guests.note(format!(
                "{reference} wasn't run: no package called {} runs now",
                self.package
            ));
            return None;
        };
        if extension.provides.filters.get(&self.filter) != Some(&self.stage) {
            self.guests.note(format!(
                "{reference} wasn't run: {} provides no {} called {}",
                self.package,
                self.stage.as_str(),
                self.filter
            ));
            return None;
        }
        let bridge = self.guests.bridge.get()?;
        let mut permits = Vec::new();
        for request in self.requests() {
            match bridge.authorize(&extension.package, &request, Trigger::Query) {
                Ok(permit) => permits.push(permit),
                Err(refused) => {
                    self.guests.note(format!("{reference} wasn't run: {refused}"));
                    return None;
                }
            }
        }
        let input = FilterInput::new(
            self.stage,
            Some(query.to_owned()),
            titles.iter().map(|title| (*title).to_owned()),
        );
        let hits = match extension.filter(&self.filter, &input, Instant::now() + FILTER_DEADLINE) {
            Ok(hits) => hits,
            Err(dupin_js_runtime::RunError::Failed(diagnostic)) => {
                self.guests.note(format!(
                    "{reference} failed: {} ({})",
                    diagnostic.message, diagnostic.code
                ));
                return None;
            }
            Err(dupin_js_runtime::RunError::Cancelled) => return None,
        };
        // Access withdrawn while it ran drops what it answered.
        if !permits.iter().all(|permit| bridge.validate(permit)) {
            return None;
        }
        Some(
            hits.into_iter()
                .map(|hit| Hit {
                    index: hit.index as usize,
                    score: hit.score,
                    positions: hit
                        .positions
                        .into_iter()
                        .map(|position| position as usize)
                        .collect(),
                })
                .collect(),
        )
    }
}

/// The kind of row `subject` stands for, as the broker names it.
pub(crate) fn row_kind(subject: &Subject) -> RowKind {
    match subject {
        Subject::Text(_) => RowKind::Text,
        Subject::Url(_) => RowKind::Url,
        Subject::File { .. } => RowKind::File,
        Subject::App(_) => RowKind::App,
    }
}

/// `field` of the row `subject` stands for, as a kind action receives it; none for a field it
/// lacks, or a path that isn't UTF-8, which would reach the package as some other path.
pub(crate) fn row_field(subject: &Subject, field: RowField) -> Option<String> {
    match (subject, field) {
        (Subject::Text(text), RowField::Text) => Some(text.clone()),
        (Subject::Url(url), RowField::Url) => Some(url.clone()),
        (Subject::File { path, .. } | Subject::App(path), RowField::Path) => path.to_str().map(str::to_owned),
        (Subject::File { path, .. } | Subject::App(path), RowField::Name) => {
            path.file_name()?.to_str().map(str::to_owned)
        }
        _ => None,
    }
}

/// One line for each part of `advanced`, as Settings shows it.
pub fn summary(advanced: &Advanced) -> Vec<String> {
    let mut lines = Vec::new();
    for (name, stages) in &advanced.sources {
        let parts: Vec<_> = [("matcher", &stages.matcher), ("sorter", &stages.sorter)]
            .into_iter()
            .filter_map(|(stage, value)| Some(format!("{stage} {}", value.as_deref()?)))
            .collect();
        lines.push(format!("{name}: {}", parts.join(", ")));
    }
    let appended = &advanced.commands.default.append_sources;
    if !appended.is_empty() {
        lines.push(format!("Main search also searches {}", appended.join(", ")));
    }
    let kinds = &advanced.kinds;
    for (kind, actions) in [
        ("Text", &kinds.text.actions),
        ("URL", &kinds.url.actions),
        ("File", &kinds.file.actions),
        ("App", &kinds.app.actions),
    ] {
        if !actions.is_empty() {
            let titles: Vec<_> = actions.iter().map(|action| action.title.as_str()).collect();
            lines.push(format!("{kind} actions: {}", titles.join(", ")));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use dupin_core::Config;

    use super::*;

    fn advanced(source: &str) -> Advanced {
        Config::parse(source).unwrap().advanced
    }

    #[test]
    fn absent_advanced_composes_exactly_what_the_manifest_registers() {
        let composition = Composition::native(dupin_mac::kinds::native());
        assert!(!composition.apply(&Advanced::default()).any());
        for source in features::sources() {
            let Some(matcher) = source.matcher else {
                assert!(!composition.sources.contains_key(source.name));
                continue;
            };
            let stages = composition.rows(source.name).filters.current();
            assert_eq!(
                (stages.matcher.name(), stages.sorter),
                (matcher, Sorter::Relevance),
                "{}",
                source.name
            );
        }
        check(&Advanced::default()).unwrap();
    }

    #[test]
    fn a_source_takes_its_configured_stages_and_returns_to_its_own_without_them() {
        let composition = Composition::native(dupin_mac::kinds::native());
        let rows = composition.rows("apps");
        let folder = composition.rows("folder");
        let configured = advanced(
            "[advanced.sources.apps]\nmatcher = \"builtin.prefix\"\n[advanced.sources.folder]\nsorter = \"builtin.title\"",
        );
        check(&configured).unwrap();
        assert_eq!(
            composition.apply(&configured),
            Recomposed {
                sources: true,
                kinds: false
            }
        );
        assert!(!composition.apply(&configured).any());
        let apps = rows.filters.current();
        assert_eq!(
            (apps.matcher.name(), apps.sorter),
            ("builtin.prefix", Sorter::Relevance)
        );
        let browsed = folder.filters.current();
        assert_eq!(
            (browsed.matcher.name(), browsed.sorter),
            ("builtin.fuzzy", Sorter::Title)
        );
        assert!(composition.apply(&Advanced::default()).sources);
        assert_eq!(rows.filters.current().matcher.name(), "builtin.fuzzy");
        assert_eq!(folder.filters.current().sorter, Sorter::Relevance);
    }

    #[test]
    fn configured_kind_actions_reach_rows_built_from_the_composition() {
        let composition = Composition::native(dupin_mac::kinds::native());
        let rows = composition.rows("folder");
        assert_eq!(
            composition.apply(&advanced(
                "[[advanced.kinds.file.actions]]\ntitle = \"Open in Zed\"\nscript = \"open-in-zed\""
            )),
            Recomposed {
                sources: false,
                kinds: true
            }
        );
        let item = rows.kinds.file("/tmp/a".into(), "a".into(), vec![], false);
        assert_eq!(
            item.action(&dupin_core::kind::configured_id("file", 0)),
            Some(&dupin_core::Action::ConfiguredScript {
                script: "open-in-zed".into(),
                argument: "/tmp/a".into()
            })
        );
    }

    #[test]
    fn settings_list_the_composition_and_nothing_without_one() {
        assert!(summary(&Advanced::default()).is_empty());
        let configured = advanced(
            r#"
[advanced.sources.apps]
matcher = "builtin.prefix"
sorter = "builtin.title"
[advanced.commands.default]
append_sources = ["files"]
[[advanced.kinds.file.actions]]
title = "Open in Zed"
script = "open-in-zed"
[[advanced.kinds.text.actions]]
title = "Translate"
url = "https://translate.example/?q={text}"
"#,
        );
        assert_eq!(
            summary(&configured),
            [
                "apps: matcher builtin.prefix, sorter builtin.title",
                "Main search also searches files",
                "Text actions: Translate",
                "File actions: Open in Zed",
            ]
        );
    }

    #[test]
    fn the_main_search_appends_configured_sources_after_its_own() {
        let defaults = |source: &str| {
            let config = Config::parse(source).unwrap();
            let configured = crate::commands::configured(&config).unwrap();
            let providers: Vec<_> = configured
                .registry
                .default_providers()
                .iter()
                .map(|id| id.as_str().to_owned())
                .collect();
            (providers, configured.notice)
        };
        let (before, notice) = defaults("");
        assert_eq!(notice, None);
        let (after, notice) =
            defaults("[advanced.commands.default]\nappend_sources = [\"files.contents\", \"files\"]");
        assert_eq!(notice, None);
        assert_eq!(after[..before.len()], before[..]);
        assert_eq!(after[before.len()..], ["files.contents", "files.names"]);
        let (shown, _) = defaults("[advanced.commands.default]\nappend_sources = [\"recent\"]");
        assert_eq!(shown, before, "recent items are already part of the main search");
        let (around, _) = defaults(
            "[advanced.commands.default]\nappend_sources = [\"files.contents\", \"recent\", \"files\"]",
        );
        assert_eq!(
            around[before.len()..],
            ["files.contents", "files.names"],
            "a source the main search already has leaves the order alone"
        );
        let (off, notice) =
            defaults("show_recent = false\n[advanced.commands.default]\nappend_sources = [\"recent\"]");
        assert!(!off.iter().any(|id| id == "recent.items"));
        assert!(notice.unwrap().contains("names `recent`, whose feature is off"));
    }

    const KANA: &str = r#"
export function search() { return []; }
export const filters = {
  romaji(input) {
    return input.candidates
      .map((candidate, index) => ({ index, score: candidate.title.length, title: candidate.title.toLowerCase() }))
      .filter((found) => found.title.includes(input.query))
      .map(({ index, score }) => ({ index, score }));
  },
  longest(input) {
    return input.candidates
      .map((candidate, index) => [candidate.title.length, index])
      .sort((a, b) => b[0] - a[0])
      .map(([, index]) => index);
  },
  spin() { for (;;) {} },
};
"#;

    fn kana(capabilities: &str, provides: &str) -> (tempfile::TempDir, Arc<Extension>) {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(
            folder.path().join("package.json"),
            format!(
                r#"{{"name": "kana", "version": "0.1.0", "dupin": {{"title": "Kana", "keyword": "kana", "capabilities": {capabilities}, "provides": {provides}}}}}"#
            ),
        )
        .unwrap();
        std::fs::write(folder.path().join("main.js"), KANA).unwrap();
        let extension = crate::extensions::load_folder(folder.path()).unwrap();
        (folder, extension)
    }

    fn allow(bridge: &Bridge, extension: &Extension, key: &str) {
        let scope = extension.capabilities.get(&key.parse().unwrap()).unwrap().clone();
        bridge
            .allow(&extension.package, &extension.revision, &scope, None)
            .unwrap();
    }

    #[test]
    fn a_packages_filter_sees_titles_and_the_query_only_with_consent_and_falls_back_without() {
        use dupin_extension_bridge::{Base, Broker, Local, Services};

        dupin_extension_host::skip_inside_a_sandbox!();
        let (_folder, extension) = kana(
            r#"{ "rows": { "app": ["title"] }, "query": ["read"] }"#,
            r#"{ "filters": { "romaji": { "stage": "matcher" }, "longest": { "stage": "sorter" }, "spin": { "stage": "sorter" } } }"#,
        );
        let bridge = Bridge::new(
            Broker::new(Base::default(), Local::default()),
            None,
            Services::none(),
        );
        bridge.install(
            &extension.package,
            extension.revision.clone(),
            extension.capabilities.clone(),
        );
        bridge.set_enabled(&extension.package, true).unwrap();
        let composition = Composition::native(dupin_mac::kinds::native());
        let apps = composition.rows("apps");
        let stages = || apps.filters.current();
        composition.apply(&advanced(
            "[advanced.sources.apps]\nmatcher = \"kana/romaji\"\nsorter = \"kana/longest\"",
        ));
        let titles = ["Safari", "Notes", "Safari Technology Preview"];
        assert_eq!(stages().batch_matches("saf", &titles), None, "no bridge yet");
        composition.guests().set_bridge(bridge.clone());
        assert_eq!(stages().batch_matches("saf", &titles), None, "not loaded yet");
        composition.guests().set_extensions(vec![extension.clone()]);
        assert_eq!(stages().batch_matches("saf", &titles), None, "nothing granted");
        assert!(
            bridge
                .pending(Some(&extension.package))
                .iter()
                .any(|request| request.key().to_string() == "rows:app"),
            "the refusal waits in Settings"
        );
        allow(&bridge, &extension, "rows:app");
        assert_eq!(stages().batch_order(&titles), Some(vec![2, 0, 1]));
        assert_eq!(
            stages().batch_matches("saf", &titles),
            None,
            "a matcher also needs the query"
        );
        allow(&bridge, &extension, "query");
        let scores: Vec<_> = stages()
            .batch_matches("saf", &titles)
            .unwrap()
            .into_iter()
            .map(|found| found.map(|found| found.score))
            .collect();
        assert_eq!(scores, [Some(6), None, Some(25)]);

        composition.apply(&advanced("[advanced.sources.apps]\nsorter = \"kana/spin\""));
        let started = Instant::now();
        assert_eq!(stages().batch_order(&titles), None, "an overrun falls back");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );

        composition.apply(&advanced("[advanced.sources.apps]\nmatcher = \"kana/longest\""));
        assert_eq!(
            stages().batch_matches("saf", &titles),
            None,
            "a sorter can't stand in for a matcher"
        );
        composition.apply(&advanced("[advanced.sources.apps]\nmatcher = \"kana/romaji\""));
        bridge
            .revoke(&extension.package, &"rows:app".parse().unwrap())
            .unwrap();
        assert_eq!(stages().batch_matches("saf", &titles), None, "revoked");
        let many: Vec<String> = (0..=dupin_js_runtime::filter::MAX_CANDIDATES)
            .map(|index| index.to_string())
            .collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        allow(&bridge, &extension, "rows:app");
        assert_eq!(
            stages().batch_matches("1", &many),
            None,
            "a source with more candidates than a batch takes keeps its own matcher"
        );
    }

    #[test]
    fn a_packages_kind_action_reaches_only_rows_dupin_found_and_hands_one_field() {
        use dupin_core::kind::{Producer, Subject};
        use dupin_core::{Action, ExtensionRequest};

        let composition = Composition::native(dupin_mac::kinds::native());
        composition.apply(&advanced(
            "[[advanced.kinds.file.actions]]\ntitle = \"Upload\"\nextension = \"uploader/upload\"\n[[advanced.kinds.text.actions]]\ntitle = \"Send\"\nextension = \"uploader/send\"",
        ));
        let file = Subject::File {
            path: "/Users/test/a.txt".into(),
            folder: false,
        };
        let native = composition.kinds().configured(&file, Producer::Dupin);
        assert_eq!(
            native,
            [dupin_core::ItemAction::new(
                "configured-file-0",
                "Upload",
                Action::Extension {
                    extension: "uploader".into(),
                    request: ExtensionRequest::Kind {
                        action: "upload".into(),
                        subject: file.clone(),
                    },
                },
            )
            .keep_open()]
        );
        let text = Subject::Text("an extension's text".into());
        assert!(
            composition
                .kinds()
                .configured(&text, Producer::Extension)
                .is_empty(),
            "another extension's rows never reach a package's kind action"
        );
        assert_eq!(composition.kinds().configured(&text, Producer::Dupin).len(), 1);
        assert_eq!(row_kind(&file), RowKind::File);
        assert_eq!(row_field(&file, RowField::Name).as_deref(), Some("a.txt"));
        assert_eq!(
            row_field(&file, RowField::Path).as_deref(),
            Some("/Users/test/a.txt")
        );
        assert_eq!(row_field(&file, RowField::Url), None);
        let unnamed = Subject::App(std::path::PathBuf::from(
            <std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(
                b"/Applications/\xff.app".to_vec(),
            ),
        ));
        assert_eq!(row_field(&unnamed, RowField::Path), None);
    }

    #[test]
    fn names_dupin_lacks_and_unsupported_stages_are_rejected_by_name() {
        for (source, expected) in [
            (
                "[advanced.sources.aps]\nmatcher = \"builtin.prefix\"",
                "[advanced.sources.aps] names `aps`, which isn't a source Dupin has (files, files.contents, folder, apps, recent)",
            ),
            (
                "[advanced.sources.files]\nmatcher = \"builtin.prefix\"",
                "[advanced.sources.files]: `files` matches through an index of its own",
            ),
            (
                "[advanced.sources.apps]\nmatcher = \"builtin.prefx\"",
                "`builtin.prefx` isn't a matcher Dupin has (builtin.fuzzy, builtin.prefix)",
            ),
            (
                "[advanced.sources.apps]\nsorter = \"builtin.random\"",
                "`builtin.random` isn't a sorter Dupin has (builtin.relevance, builtin.title)",
            ),
            (
                "[advanced.commands.default]\nappend_sources = [\"bookmarks\"]",
                "append_sources names `bookmarks`, which isn't a source Dupin has",
            ),
            (
                "[advanced.commands.default]\nappend_sources = [\"apps\"]",
                "which is the main search's own source",
            ),
            (
                "[advanced.commands.default]\nappend_sources = [\"folder\"]",
                "which lists only a folder being browsed",
            ),
        ] {
            let problem = check(&advanced(source)).unwrap_err();
            assert!(problem.contains(expected), "{source:?}: {problem}");
        }
        check(&advanced(
            "[advanced.commands.default]\nappend_sources = [\"files\", \"recent\"]",
        ))
        .unwrap();
    }
}
