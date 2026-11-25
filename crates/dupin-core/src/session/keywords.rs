//! Rows for the commands the main search's query is a keyword of, or the start of one, so a
//! keyword can be found by typing it.

use std::collections::HashMap;

use crate::{Accessory, Action, CommandId, Item, ItemAction, KeywordEntry, KeywordTarget, Payload, View};

use super::search::Presentation;
use super::{SearchLocation, Session};

/// The origin of these rows, which no provider answers.
const KEYWORDS: &str = "launcher.keywords";
/// The query that lists every command and its keywords.
const LIST: &str = "?";
/// The most rows a query that is not [`LIST`] adds.
const MAX_SUGGESTIONS: usize = 3;
/// Shorter than this, an exact keyword leads the results only when no other command's
/// keyword starts with the query, so one or two letters keep the best app first.
const LEADING_CHARS: usize = 3;
const MAX_TAGS: usize = 3;
pub(crate) const ACTION: &str = "keyword";

pub(super) fn origin() -> CommandId {
    CommandId::new(KEYWORDS).expect("a valid command ID")
}

/// A command the query reaches, and the keyword that does.
struct Suggestion {
    entry: KeywordEntry,
    keyword: String,
    exact: bool,
}

impl Session {
    fn suggests(&self) -> bool {
        !self.search.explicit
            && self.destination.is_none()
            && matches!(self.search.location, SearchLocation::Command)
            && self.search.command == *self.commands.default()
    }

    /// Works out the rows for the current query, before any provider answers it.
    pub(super) fn suggest(&mut self) {
        self.search.suggestions.clear();
        self.search.leading = 0;
        self.search.presentations.remove(&origin());
        if !self.suggests() {
            return;
        }
        let query = self.search.query.trim();
        if query.is_empty() || query.contains(char::is_whitespace) {
            return;
        }
        let (suggestions, leading) = if query == LIST {
            let listed: Vec<_> = self
                .commands
                .entries()
                .into_iter()
                .map(|entry| Suggestion {
                    keyword: entry.keywords.first().cloned().unwrap_or_default(),
                    entry,
                    exact: false,
                })
                .collect();
            let leading = listed.len();
            (listed, leading)
        } else {
            matching(self.commands.entries(), query)
        };
        let mut rows = HashMap::new();
        for suggestion in suggestions {
            let (item, tags) = row(suggestion);
            rows.insert(
                item.id.clone(),
                Presentation {
                    section: None,
                    detail: None,
                    detail_when_selected: false,
                    accessories: tags,
                    form: None,
                },
            );
            self.search.suggestions.push(item);
        }
        self.search.leading = leading;
        if !rows.is_empty() {
            self.search.presentations.insert(origin(), rows);
        }
    }
}

/// The commands whose keywords start with `query`, best first, and how many lead the results.
///
/// An exact keyword ranks first and leads once the query has [`LEADING_CHARS`] characters, or
/// sooner when no other command's keyword starts with it (`d`, `in`); a keyword the query only
/// starts follows the best result. A web search's own alias already shows its row first, so it
/// adds none.
fn matching(entries: Vec<KeywordEntry>, query: &str) -> (Vec<Suggestion>, usize) {
    let starts = |keyword: &str| {
        keyword
            .get(..query.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(query))
    };
    let mut reached = 0;
    let mut suggestions: Vec<Suggestion> = entries
        .into_iter()
        .filter_map(|entry| {
            let keyword = entry
                .keywords
                .iter()
                .filter(|keyword| starts(keyword))
                .min_by_key(|keyword| keyword.len())?
                .clone();
            reached += 1;
            let exact = keyword.len() == query.len();
            if exact && matches!(entry.target, KeywordTarget::Web(_)) {
                return None;
            }
            Some(Suggestion {
                entry,
                keyword,
                exact,
            })
        })
        .collect();
    suggestions.sort_by(|a, b| {
        b.exact
            .cmp(&a.exact)
            .then_with(|| a.keyword.len().cmp(&b.keyword.len()))
            .then_with(|| a.entry.title.cmp(&b.entry.title))
    });
    suggestions.truncate(MAX_SUGGESTIONS);
    let leads = suggestions
        .first()
        .is_some_and(|first| first.exact && (query.chars().count() >= LEADING_CHARS || reached == 1));
    (suggestions, usize::from(leads))
}

/// The row for `suggestion`, and the keywords it shows as tags, the one typed first.
fn row(suggestion: Suggestion) -> (Item, Vec<Accessory>) {
    let Suggestion { entry, keyword, .. } = suggestion;
    let mut keywords = vec![keyword.clone()];
    keywords.extend(entry.keywords.iter().filter(|other| **other != keyword).cloned());
    let tags = keywords
        .iter()
        .take(MAX_TAGS)
        .map(|keyword| Accessory::Tag(keyword_tag(keyword)))
        .collect();
    let item = Item {
        id: entry.target.to_string(),
        view: View::Text {
            title: entry.title,
            subtitle: entry.subtitle,
        },
        payload: Payload::default(),
        record_usage: false,
        icon: entry.icon,
        matched: Vec::new(),
        completion: None,
        score: None,
        primary: Some(ACTION.into()),
        alternate: None,
        actions: vec![ItemAction::new(ACTION, "Use Keyword", Action::Keyword(keyword))],
    };
    (item, tags)
}

/// How a row shows a keyword: what to type, then Space.
pub fn keyword_tag(keyword: &str) -> String {
    if keyword.is_empty() {
        "␣".into()
    } else {
        format!("{keyword} ␣")
    }
}

#[cfg(test)]
mod tests;
