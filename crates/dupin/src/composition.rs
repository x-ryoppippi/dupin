//! What `[advanced]` composes over the feature manifest: each native source's filter stages,
//! the actions added to the kinds and the sources the main search appends. Every source and
//! row is built from one composition, so a reload reaches them all at once.

use std::collections::BTreeMap;

use dupin_core::config::Advanced;
use dupin_core::filter::{Filters, Sorter, SourceFilters, Stages};
use dupin_core::kind::Kinds;
use dupin_mac::kinds::Rows;

use crate::features::{self, Source};

pub struct Composition {
    kinds: Kinds,
    filters: Filters,
    /// The stages of each source that matches its own candidates.
    sources: BTreeMap<&'static str, SourceFilters>,
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
        }
    }

    pub fn kinds(&self) -> &Kinds {
        &self.kinds
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
                if let Some(matcher) = configured
                    .matcher
                    .as_deref()
                    .and_then(|name| self.filters.matcher(name))
                {
                    stages.matcher = matcher;
                }
                if let Some(sorter) = configured.sorter.as_deref().and_then(Sorter::named) {
                    stages.sorter = sorter;
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
    Some(Stages {
        matcher: filters.matcher(source.matcher?)?,
        sorter: Sorter::Relevance,
    })
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
        {
            return Err(format!(
                "{table}: `{matcher}` isn't a matcher Dupin has ({})",
                filters.matcher_names().collect::<Vec<_>>().join(", ")
            ));
        }
        if let Some(sorter) = &stages.sorter
            && Sorter::named(sorter).is_none()
        {
            return Err(format!(
                "{table}: `{sorter}` isn't a sorter Dupin has ({})",
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
        let (off, notice) =
            defaults("show_recent = false\n[advanced.commands.default]\nappend_sources = [\"recent\"]");
        assert!(!off.iter().any(|id| id == "recent.items"));
        assert!(notice.unwrap().contains("names `recent`, whose feature is off"));
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
