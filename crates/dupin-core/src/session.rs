//! The launcher session: which mode the panel is in, the results it shows and which one is
//! selected. The UI forwards query edits and key intents; the session answers with effects
//! (start a search, run an action, hide) for the UI and platform layers to carry out.

use std::collections::VecDeque;

use crate::searcher::Delivery;
use crate::{Action, History, Item, ItemKind};

/// Which provider the query goes to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Fuzzy search over installed apps.
    #[default]
    Apps,
    /// Spotlight search by file name, entered with Space on an empty query (as Alfred does).
    Files,
    /// Spotlight search inside files, entered by typing the keyword `in` and Space (Alfred's
    /// `in`).
    Contents,
    /// Look up a term in macOS dictionaries, entered by typing `define` and Space.
    Dictionary,
}

/// Typed before Space in the apps search, enters [`Mode::Contents`].
pub const CONTENTS_KEYWORD: &str = "in";

const QUERY_HISTORY_LIMIT: usize = 100;

/// Keys the session gives meaning to. The UI maps raw keys onto these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Space typed without modifiers.
    Space,
    Backspace,
    Up,
    Down,
    /// Enter.
    Open,
    /// ⌘Enter.
    OpenAlternate,
    Actions,
    /// Esc.
    Dismiss,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    Search {
        mode: Mode,
        generation: u64,
        query: String,
    },
    /// Stop whatever search is running; nothing should arrive for the current results.
    CancelSearch,
    Run(Action),
    ShowActions {
        id: String,
        generation: u64,
    },
    /// Hide the panel. The session has already reset itself for the next show.
    Hide,
    /// The selection moved; keep the selected row on screen.
    RevealSelection,
    /// Empty the text field: a mode keyword was consumed. The session's query is already empty.
    ClearQuery,
    /// Replace the input with the session's query after routing or recall.
    ReplaceQuery(String),
    /// The history learned a pick; persist `Session::history`.
    SaveHistory,
}

/// Where the results on screen stand relative to the current query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Results {
    /// Nothing to search for (an empty query).
    Idle,
    /// A search is running; the rows shown, if any, belong to the previous query until its
    /// first batch arrives, so typing doesn't flash an empty list.
    Searching { replaced: bool },
    /// The search finished; the rows are all there is.
    Done,
}

#[derive(Debug)]
pub struct Session {
    mode: Mode,
    query: String,
    items: Vec<Item>,
    selected: usize,
    generation: u64,
    results: Results,
    refresh_selection: Option<String>,
    history: History,
    /// Unix seconds; a parameter so tests can fix the time.
    clock: fn() -> u64,
    dictionary_keywords: Vec<String>,
    queries: VecDeque<(Mode, String)>,
    recalled: Option<usize>,
    deleting: bool,
    before_deletion: Option<String>,
}

impl Default for Session {
    fn default() -> Self {
        Session::new(History::default())
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Session {
    /// A session that ranks with, and learns into, `history`.
    pub fn new(history: History) -> Session {
        Self::with_dictionary_keywords(history, crate::Config::default().dictionary_keywords)
    }

    /// A session with configured keywords for macOS Dictionary search.
    pub fn with_dictionary_keywords(history: History, dictionary_keywords: Vec<String>) -> Session {
        Session {
            mode: Mode::Apps,
            query: String::new(),
            items: Vec::new(),
            selected: 0,
            generation: 0,
            results: Results::Idle,
            refresh_selection: None,
            history,
            clock: unix_now,
            dictionary_keywords,
            queries: VecDeque::new(),
            recalled: None,
            deleting: false,
            before_deletion: None,
        }
    }

    pub fn set_dictionary_keywords(&mut self, dictionary_keywords: Vec<String>) {
        self.dictionary_keywords = dictionary_keywords;
    }

    /// The same, with a clock in Unix seconds other than the system's.
    pub fn with_clock(history: History, clock: fn() -> u64) -> Session {
        Session {
            clock,
            ..Session::new(history)
        }
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn selected_item(&self) -> Option<&Item> {
        self.items.get(self.selected)
    }

    fn actions(&self) -> Vec<Effect> {
        self.selected_item()
            .filter(|item| !item.actions.is_empty())
            .map(|item| {
                vec![Effect::ShowActions {
                    id: item.id.clone(),
                    generation: self.generation,
                }]
            })
            .unwrap_or_default()
    }

    pub fn run_action(&mut self, generation: u64, item_id: &str, action_id: &str) -> Vec<Effect> {
        if generation != self.generation {
            return Vec::new();
        }
        let Some(item) = self.items.iter().find(|item| item.id == item_id).cloned() else {
            return Vec::new();
        };
        let Some(action) = item.action(action_id).cloned() else {
            return Vec::new();
        };
        self.activate(&item, action)
    }

    fn activate(&mut self, item: &Item, action: Action) -> Vec<Effect> {
        let mut effects = vec![Effect::Run(action)];
        if item.kind != ItemKind::Dictionary {
            self.history.record(&item.id, &self.query, (self.clock)());
            effects.push(Effect::SaveHistory);
        }
        self.remember_query(true);
        effects.extend(self.dismiss());
        effects
    }

    pub fn results(&self) -> Results {
        self.results
    }

    /// Select the result with this id, as a click on its row does. False when it's gone.
    pub fn select_id(&mut self, id: &str) -> bool {
        let Some(i) = self.items.iter().position(|item| item.id == id) else {
            return false;
        };
        self.selected = i;
        true
    }

    /// The text field's contents changed.
    pub fn set_query(&mut self, text: &str) -> Vec<Effect> {
        if text == self.query {
            return Vec::new();
        }
        let mut previous = self.query.chars();
        let deletion = text.len() < self.query.len()
            && text
                .chars()
                .all(|ch| previous.by_ref().any(|before| before == ch));
        if deletion {
            if !self.deleting {
                self.before_deletion = (self.results == Results::Done).then(|| self.query.trim().to_owned());
                self.deleting = true;
            }
        } else {
            self.deleting = false;
            self.before_deletion = None;
        }
        if text.trim().is_empty() {
            self.remember_query(false);
            self.deleting = false;
        }
        self.recalled = None;
        if self.mode == Mode::Apps
            && let Some((mode, term)) = self.keyword_query(text)
        {
            self.deleting = false;
            self.before_deletion = None;
            self.items.clear();
            self.selected = 0;
            self.query = term.to_owned();
            let mut effects = vec![Effect::ReplaceQuery(self.query.clone())];
            effects.extend(self.switch(mode));
            return effects;
        }
        self.query = text.to_owned();
        self.search()
    }

    fn keyword_mode(&self, keyword: &str) -> Option<Mode> {
        if keyword == CONTENTS_KEYWORD {
            Some(Mode::Contents)
        } else if self.dictionary_keywords.iter().any(|alias| alias == keyword) {
            Some(Mode::Dictionary)
        } else {
            None
        }
    }

    fn keyword_query<'a>(&self, text: &'a str) -> Option<(Mode, &'a str)> {
        text.char_indices()
            .rev()
            .filter(|(_, ch)| ch.is_whitespace())
            .find_map(|(end, _)| {
                self.keyword_mode(&text[..end])
                    .map(|mode| (mode, text[end..].trim_start()))
            })
    }

    /// Search the current query again after a provider's underlying data changes.
    pub fn refresh(&mut self) -> Vec<Effect> {
        let selected = self.selected_item().map(|item| item.id.clone());
        let effects = self.search();
        self.refresh_selection = selected;
        effects
    }

    /// A key the UI recognized. None leaves the key to the text field (typing a space into a
    /// query, deleting a character); Some consumes it, even when nothing else happens.
    pub fn intent(&mut self, intent: Intent) -> Option<Vec<Effect>> {
        let empty = self.query.is_empty();
        Some(match intent {
            Intent::Actions => self.actions(),
            Intent::Space if empty => match self.mode {
                Mode::Apps => self.switch(Mode::Files),
                // A leading space finds nothing in a file search; swallow it.
                Mode::Files | Mode::Contents | Mode::Dictionary => Vec::new(),
            },
            Intent::Space if self.mode == Mode::Apps => {
                let mode = self.keyword_mode(&self.query)?;
                self.query.clear();
                let mut effects = vec![Effect::ClearQuery];
                effects.extend(self.switch(mode));
                effects
            }
            Intent::Backspace if empty => match self.mode {
                Mode::Files | Mode::Contents | Mode::Dictionary => self.switch(Mode::Apps),
                Mode::Apps => return None,
            },
            Intent::Space | Intent::Backspace => return None,
            Intent::Up if empty || self.recalled.is_some() => self.recall(true),
            Intent::Down if self.recalled.is_some() => self.recall(false),
            Intent::Up | Intent::Down => {
                let n = self.items.len();
                if n == 0 {
                    return Some(Vec::new());
                }
                self.selected = if intent == Intent::Down {
                    (self.selected + 1) % n
                } else {
                    (self.selected + n - 1) % n
                };
                vec![Effect::RevealSelection]
            }
            Intent::Open | Intent::OpenAlternate => {
                let Some(item) = self.selected_item() else {
                    return Some(Vec::new());
                };
                let id = if intent == Intent::Open {
                    item.primary.as_deref()
                } else {
                    item.alternate.as_deref()
                };
                let Some(id) = id else {
                    return Some(if intent == Intent::Open {
                        self.actions()
                    } else {
                        Vec::new()
                    });
                };
                let Some(action) = item.action(id).cloned() else {
                    return Some(Vec::new());
                };
                let item = item.clone();
                self.activate(&item, action)
            }
            Intent::Dismiss => self.dismiss(),
        })
    }

    /// A provider's answer. Returns whether anything visible changed.
    pub fn deliver(&mut self, delivery: Delivery) -> bool {
        if delivery.generation() != self.generation {
            return false;
        }
        match (delivery, self.results) {
            (Delivery::Batch { items, .. }, Results::Searching { replaced: false }) => {
                self.items = items;
                self.history.rerank(&mut self.items, &self.query, (self.clock)());
                self.selected = 0;
                if let Some(id) = self.refresh_selection.take() {
                    self.select_id(&id);
                }
                self.results = Results::Searching { replaced: true };
            }
            (Delivery::Batch { items, .. }, Results::Searching { replaced: true }) => {
                // Keep the selection on the same item while the list reorders.
                let selected = self.selected_item().map(|item| item.id.clone());
                self.items.extend(items);
                self.history.rerank(&mut self.items, &self.query, (self.clock)());
                if let Some(id) = selected {
                    self.select_id(&id);
                }
            }
            (Delivery::Done { .. }, Results::Searching { replaced }) => {
                if !replaced {
                    self.items.clear();
                    self.selected = 0;
                }
                self.results = Results::Done;
            }
            // Late answers for a search that already finished or was abandoned.
            (_, Results::Idle | Results::Done) => return false,
        }
        true
    }

    /// Back to an empty apps search, as the panel should look the next time it shows.
    pub fn reset(&mut self) -> Vec<Effect> {
        self.remember_query(false);
        self.recalled = None;
        self.deleting = false;
        self.before_deletion = None;
        let was_searching = matches!(self.results, Results::Searching { .. });
        self.mode = Mode::Apps;
        self.query.clear();
        self.clear_results();
        if was_searching {
            vec![Effect::CancelSearch]
        } else {
            Vec::new()
        }
    }

    fn dismiss(&mut self) -> Vec<Effect> {
        let mut effects = self.reset();
        effects.push(Effect::Hide);
        effects
    }

    fn remember_query(&mut self, activated: bool) {
        if activated {
            self.deleting = false;
            self.before_deletion = None;
        } else if let Some(query) = self.before_deletion.take() {
            self.record_query(query);
            return;
        }
        let query = self.query.trim();
        if (!activated && self.results != Results::Done) || query.is_empty() {
            return;
        }
        self.record_query(query.to_owned());
    }

    fn record_query(&mut self, query: String) {
        let previous = self.queries.iter().rev().find(|(mode, _)| *mode == self.mode);
        if previous.is_some_and(|(_, text)| *text == query) {
            return;
        }
        self.queries.push_back((self.mode, query));
        if self.queries.len() > QUERY_HISTORY_LIMIT {
            self.queries.pop_front();
        }
    }

    fn recall(&mut self, backwards: bool) -> Vec<Effect> {
        let next = if backwards {
            let end = self.recalled.unwrap_or(self.queries.len());
            (0..end).rev().find(|&i| self.queries[i].0 == self.mode)
        } else {
            (self.recalled.unwrap() + 1..self.queries.len()).find(|&i| self.queries[i].0 == self.mode)
        };
        if backwards && next.is_none() {
            return Vec::new();
        }
        self.recalled = next;
        self.deleting = false;
        self.before_deletion = None;
        self.query = next.map_or_else(String::new, |i| self.queries[i].1.clone());
        let mut effects = vec![Effect::ReplaceQuery(self.query.clone())];
        effects.extend(self.search());
        effects
    }

    fn switch(&mut self, mode: Mode) -> Vec<Effect> {
        self.recalled = None;
        self.deleting = false;
        self.before_deletion = None;
        self.mode = mode;
        self.search()
    }

    fn search(&mut self) -> Vec<Effect> {
        self.refresh_selection = None;
        let query = self.query.trim();
        if query.is_empty() {
            let was_searching = matches!(self.results, Results::Searching { .. });
            self.clear_results();
            return if was_searching {
                vec![Effect::CancelSearch]
            } else {
                Vec::new()
            };
        }
        let query = query.to_owned();
        self.generation += 1;
        // A mode change clears the previous mode's rows before starting a search.
        self.results = Results::Searching { replaced: false };
        vec![Effect::Search {
            mode: self.mode,
            generation: self.generation,
            query,
        }]
    }

    fn clear_results(&mut self) {
        // Bump the generation so answers still in flight are recognized as stale.
        self.generation += 1;
        self.items.clear();
        self.selected = 0;
        self.results = Results::Idle;
    }
}

#[cfg(test)]
mod tests;
