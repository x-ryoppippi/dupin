//! Filters: the stages a source's candidates pass through before they show. A matcher picks
//! the candidates a query finds and where it found them; the source then orders the matches
//! and builds their rows. A source that matches through an index of its own, such as
//! Spotlight, takes no matcher.

use std::collections::HashMap;
use std::sync::Arc;

use crate::fuzzy::{self, Match};

/// The matcher every native source that takes one starts with.
pub const FUZZY: &str = "builtin.fuzzy";

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

/// The matchers a source may be composed with, by name.
#[derive(Clone)]
pub struct Filters {
    matchers: HashMap<&'static str, Arc<dyn Matcher>>,
}

impl Filters {
    pub fn native() -> Self {
        let mut filters = Self {
            matchers: HashMap::new(),
        };
        filters.register(Arc::new(Fuzzy));
        filters
    }

    fn register(&mut self, matcher: Arc<dyn Matcher>) {
        self.matchers.insert(matcher.name(), matcher);
    }

    pub fn matcher(&self, name: &str) -> Option<Arc<dyn Matcher>> {
        self.matchers.get(name).cloned()
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
        assert!(Filters::native().matcher("builtin.prefix").is_none());
    }
}
