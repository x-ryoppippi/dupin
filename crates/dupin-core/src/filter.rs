//! Filters: the stages a source's candidates pass through before they show. A matcher picks
//! the candidates a query finds and where it found them, and a sorter orders the matches
//! before the source builds their rows. A source that matches through an index of its own,
//! such as Spotlight, takes neither.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use crate::fuzzy::{self, Match};

/// The matcher every native source that takes one starts with.
pub const FUZZY: &str = "builtin.fuzzy";
/// Matches a text, or a word in it, that starts with the query.
const PREFIX: &str = "builtin.prefix";

/// Decides whether a query finds a text, how well, and which characters it matched.
pub trait Matcher: Send + Sync + 'static {
    /// The name configuration addresses it by.
    fn name(&self) -> &'static str;

    fn score(&self, query: &str, text: &str) -> Option<Match>;
}

/// Dupin's fuzzy matcher, which ranks prefixes, word starts and runs above scattered letters.
struct Fuzzy;

impl Matcher for Fuzzy {
    fn name(&self) -> &'static str {
        FUZZY
    }

    fn score(&self, query: &str, text: &str) -> Option<Match> {
        fuzzy::score(query, text)
    }
}

/// Case-insensitive prefixes of the text or of one of its words. A prefix of the whole text
/// ranks above a word's, and a shorter text above a longer one.
struct Prefix;

const WHOLE_PREFIX: i32 = 64;

impl Matcher for Prefix {
    fn name(&self) -> &'static str {
        PREFIX
    }

    fn score(&self, query: &str, text: &str) -> Option<Match> {
        let query: Vec<char> = query.trim().chars().flat_map(char::to_lowercase).collect();
        if query.is_empty() {
            return None;
        }
        let text: Vec<char> = text.chars().collect();
        // Lowercased as the query is, each lowered character remembering the one it came from,
        // since lowering can turn one character into two.
        let (lower, from): (Vec<char>, Vec<usize>) = text
            .iter()
            .enumerate()
            .flat_map(|(index, ch)| ch.to_lowercase().map(move |lowered| (lowered, index)))
            .unzip();
        let length = i32::try_from(text.len()).unwrap_or(i32::MAX);
        (0..lower.len())
            .filter(|&start| {
                let index = from[start];
                (start == 0 || from[start - 1] != index)
                    && (index == 0 || (!text[index - 1].is_alphanumeric() && text[index].is_alphanumeric()))
            })
            .find(|&start| lower[start..].starts_with(&query))
            .map(|start| {
                let mut positions: Vec<usize> = from[start..start + query.len()].to_vec();
                positions.dedup();
                Match {
                    score: if from[start] == 0 { WHOLE_PREFIX } else { 0 } - length.min(WHOLE_PREFIX - 1),
                    positions,
                }
            })
    }
}

/// How a source orders its matches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sorter {
    /// Best matches first, and the usage history moves what was picked up.
    #[default]
    Relevance,
    /// By title, ignoring case; the usage history leaves the order alone.
    Title,
}

impl Sorter {
    pub const ALL: [Self; 2] = [Self::Relevance, Self::Title];

    /// The name configuration addresses it by.
    pub fn name(self) -> &'static str {
        match self {
            Self::Relevance => "builtin.relevance",
            Self::Title => "builtin.title",
        }
    }

    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|sorter| sorter.name() == name)
    }
}

/// A candidate a batch stage kept, by its index in the batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub index: usize,
    pub score: Option<i32>,
    /// Character indices in the candidate that matched.
    pub positions: Vec<usize>,
}

/// A matcher or sorter that takes a whole batch of a source's candidates in one call, as a
/// package's does.
pub trait BatchStage: Send + Sync + 'static {
    /// The name configuration addresses it by.
    fn name(&self) -> &str;

    /// For a matcher, the hits `query` finds among `titles`; for a sorter, every one of them
    /// in its order. None when it can't run now, so the source runs its own stage instead.
    fn run(&self, query: &str, titles: &[&str]) -> Option<Vec<Hit>>;
}

/// A source's filter stages.
#[derive(Clone)]
pub struct Stages {
    pub matcher: Arc<dyn Matcher>,
    pub sorter: Sorter,
    /// A package's matcher composed over `matcher`, which still runs whenever it can't.
    pub batch_matcher: Option<Arc<dyn BatchStage>>,
    /// A package's sorter composed over `sorter`, which still runs whenever it can't.
    pub batch_sorter: Option<Arc<dyn BatchStage>>,
}

impl Stages {
    pub fn native(matcher: Arc<dyn Matcher>, sorter: Sorter) -> Self {
        Self {
            matcher,
            sorter,
            batch_matcher: None,
            batch_sorter: None,
        }
    }

    fn same(&self, other: &Self) -> bool {
        let name = |stage: &Option<Arc<dyn BatchStage>>| stage.as_ref().map(|stage| stage.name().to_owned());
        self.matcher.name() == other.matcher.name()
            && self.sorter == other.sorter
            && name(&self.batch_matcher) == name(&other.batch_matcher)
            && name(&self.batch_sorter) == name(&other.batch_sorter)
    }

    /// Each of `texts`' match for `query` by the package's matcher, or None without one or when
    /// it can't run now.
    pub fn batch_matches(&self, query: &str, texts: &[&str]) -> Option<Vec<Option<Match>>> {
        let hits = self.batch_matcher.as_ref()?.run(query, texts)?;
        let mut matches = vec![None; texts.len()];
        for hit in hits {
            let slot = matches.get_mut(hit.index)?;
            *slot = Some(Match {
                score: hit.score.unwrap_or_default(),
                positions: hit.positions,
            });
        }
        Some(matches)
    }

    /// The order the package's sorter puts `titles` in, as indices into them, or None without
    /// one or when it can't run now.
    pub fn batch_order(&self, titles: &[&str]) -> Option<Vec<usize>> {
        let hits = self.batch_sorter.as_ref()?.run("", titles)?;
        let mut seen = vec![false; titles.len()];
        let order: Vec<usize> = hits.into_iter().map(|hit| hit.index).collect();
        let complete = order.len() == titles.len()
            && order.iter().all(|&index| {
                seen.get_mut(index)
                    .is_some_and(|seen| !std::mem::replace(seen, true))
            });
        complete.then_some(order)
    }

    /// Whether the source's rows keep the order the stages gave them, which usage history then
    /// leaves alone.
    pub fn ordered(&self) -> bool {
        self.sorter == Sorter::Title || self.batch_sorter.is_some()
    }
}

/// The stages one source runs, which a configuration reload replaces for every search after
/// it. A search reads them once, not per candidate.
#[derive(Clone)]
pub struct SourceFilters(Arc<RwLock<Stages>>);

impl SourceFilters {
    pub fn new(stages: Stages) -> Self {
        Self(Arc::new(RwLock::new(stages)))
    }

    pub fn current(&self) -> Stages {
        self.0.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Whether `stages` differ from the ones the source ran.
    pub fn set(&self, stages: Stages) -> bool {
        let mut current = self.0.write().unwrap_or_else(PoisonError::into_inner);
        let changed = !current.same(&stages);
        *current = stages;
        changed
    }
}

/// The matchers a source may be composed with, by name.
#[derive(Clone)]
pub struct Filters {
    matchers: BTreeMap<&'static str, Arc<dyn Matcher>>,
}

impl Filters {
    pub fn native() -> Self {
        let mut filters = Self {
            matchers: BTreeMap::new(),
        };
        filters.register(Arc::new(Fuzzy));
        filters.register(Arc::new(Prefix));
        filters
    }

    fn register(&mut self, matcher: Arc<dyn Matcher>) {
        self.matchers.insert(matcher.name(), matcher);
    }

    pub fn matcher(&self, name: &str) -> Option<Arc<dyn Matcher>> {
        self.matchers.get(name).cloned()
    }

    /// The registered matchers' names, in order.
    pub fn matcher_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.matchers.keys().copied()
    }
}

/// The fuzzy matcher, for a source built outside a registration.
pub fn fuzzy() -> Arc<dyn Matcher> {
    Arc::new(Fuzzy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_native_fuzzy_matcher_scores_as_the_fuzzy_scorer_does() {
        let matcher = Filters::native().matcher(FUZZY).unwrap();
        assert_eq!(matcher.name(), FUZZY);
        for (query, text) in [
            ("saf", "Safari"),
            ("", "Safari"),
            ("xyz", "Safari"),
            ("ss", "System Settings"),
        ] {
            assert_eq!(
                matcher.score(query, text),
                fuzzy::score(query, text),
                "{query} {text}"
            );
        }
        assert!(Filters::native().matcher("builtin.prefx").is_none());
    }

    #[test]
    fn the_prefix_matcher_finds_starts_of_the_text_and_its_words() {
        let prefix = Filters::native().matcher(PREFIX).unwrap();
        let found = |query: &str, text: &str| prefix.score(query, text);
        let safari = found("saf", "Safari").unwrap();
        assert_eq!(safari.positions, [0, 1, 2]);
        let word = found("saf", "Hush Safe").unwrap();
        assert_eq!(word.positions, [5, 6, 7]);
        assert!(safari.score > word.score);
        assert!(
            found("saf", "Safari").unwrap().score > found("saf", "Safari Technology Preview").unwrap().score
        );
        assert_eq!(found("SAF", "safari").unwrap().positions, [0, 1, 2]);
        for (query, text) in [
            ("afa", "Safari"),
            ("sfr", "Safari"),
            ("", "Safari"),
            ("  ", "Safari"),
            ("x", ""),
        ] {
            assert_eq!(found(query, text), None, "{query:?} {text:?}");
        }
        assert_eq!(found("set", "System Settings").unwrap().positions, [7, 8, 9]);
        assert_eq!(found("é", "Éclair").unwrap().positions, [0]);
        assert_eq!(found("İ", "İstanbul").unwrap().positions, [0]);
        assert_eq!(found("i̇s", "İstanbul").unwrap().positions, [0, 1]);
        assert_eq!(found("sta", "İstanbul"), None);
    }

    #[test]
    fn sorters_are_addressed_by_name_and_stages_report_a_change() {
        for sorter in Sorter::ALL {
            assert_eq!(Sorter::named(sorter.name()), Some(sorter));
        }
        assert_eq!(Sorter::named("builtin.random"), None);
        let filters = Filters::native();
        let stages = |matcher, sorter| Stages::native(filters.matcher(matcher).unwrap(), sorter);
        let source = SourceFilters::new(stages(FUZZY, Sorter::Relevance));
        assert!(!source.set(stages(FUZZY, Sorter::Relevance)));
        assert!(source.set(stages(PREFIX, Sorter::Relevance)));
        assert_eq!(source.current().matcher.name(), PREFIX);
        assert!(source.set(stages(PREFIX, Sorter::Title)));
        assert_eq!(source.current().sorter, Sorter::Title);
        assert_eq!(filters.matcher_names().collect::<Vec<_>>(), [FUZZY, PREFIX]);
    }
}
