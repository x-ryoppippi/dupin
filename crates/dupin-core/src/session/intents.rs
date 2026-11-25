use crate::{Action, CommandId};

use super::{Effect, Intent, QUERY_HISTORY_LIMIT, Results, SearchLocation, Session};

impl Session {
    /// The text field's contents changed.
    pub fn set_query(&mut self, text: &str) -> Vec<Effect> {
        if let Some(renaming) = &mut self.renaming {
            renaming.name = text.to_owned();
            return Vec::new();
        }
        self.update_query(text, true)
    }

    /// Set externally supplied search text without interpreting it as a command keyword.
    pub fn set_literal_query(&mut self, text: &str) -> Vec<Effect> {
        self.update_query(text, false)
    }

    fn update_query(&mut self, text: &str, route_keyword: bool) -> Vec<Effect> {
        if text == self.search.query {
            return Vec::new();
        }
        if self.destination.is_some() {
            self.search.query = text.to_owned();
            return self.search();
        }
        let mut previous = self.search.query.chars();
        let deletion = text.len() < self.search.query.len()
            && text
                .chars()
                .all(|ch| previous.by_ref().any(|before| before == ch));
        if deletion {
            if !self.search.deleting {
                self.search.before_deletion =
                    (self.search.results == Results::Done).then(|| self.search.query.clone());
                self.search.deleting = true;
            }
        } else {
            self.search.deleting = false;
            self.search.before_deletion = None;
        }
        if text.trim().is_empty() {
            self.remember_query(false);
            self.search.deleting = false;
        }
        self.search.recalled = None;
        if route_keyword
            && self.page().is_none()
            && self.search.command == *self.commands.default()
            && let Some((command, term)) = self.keyword_query(text)
        {
            self.search.deleting = false;
            self.search.before_deletion = None;
            self.search.query = term.to_owned();
            let mut effects = vec![Effect::ReplaceQuery(self.search.query.clone())];
            effects.extend(self.switch(command, true));
            return effects;
        }
        self.search.query = text.to_owned();
        self.search()
    }

    fn keyword_query<'a>(&self, text: &'a str) -> Option<(CommandId, &'a str)> {
        text.char_indices()
            .rev()
            .filter(|(_, ch)| ch.is_whitespace())
            .find_map(|(end, _)| {
                self.commands
                    .keyword(&text[..end])
                    .map(|command| (command.clone(), text[end..].trim_start()))
            })
    }

    /// A key the UI recognized. None leaves the key to the text field (typing a space into a
    /// query, deleting a character); Some consumes it, even when nothing else happens.
    pub fn intent(&mut self, intent: Intent) -> Option<Vec<Effect>> {
        if self.renaming.is_some() {
            return self.rename_intent(intent);
        }
        if self.destination.is_some() {
            return self.destination_intent(intent);
        }
        let empty = self.search.query.is_empty();
        Some(match intent {
            Intent::Actions => self.actions(None),
            Intent::Complete => {
                let item = self.actionable_items().get(self.search.selected)?;
                if let Some(Action::Keyword(keyword)) = item.action(super::keywords::ACTION) {
                    let keyword = keyword.clone();
                    self.enter_keyword(&keyword)
                } else if let Some(completion) = item.completion.clone() {
                    self.complete(&completion)
                } else {
                    let Action::Browse(folder) = item.action("browse")? else {
                        return None;
                    };
                    self.browse(folder.clone())
                }
            }
            Intent::Space if self.page().is_some() => return None,
            Intent::Space if empty => self
                .commands
                .space_on_empty(&self.search.command)
                .cloned()
                .map_or_else(Vec::new, |command| self.switch(command, true)),
            Intent::Space if self.search.command == *self.commands.default() => {
                let command = self.commands.keyword(&self.search.query)?.clone();
                self.search.query.clear();
                let mut effects = vec![Effect::ClearQuery];
                effects.extend(self.switch(command, true));
                effects
            }
            Intent::Backspace if empty && !self.visits.is_empty() => self.leave_view(),
            Intent::Backspace if empty && self.search.command != *self.commands.default() => {
                self.switch(self.commands.default().clone(), false)
            }
            Intent::Backspace if empty => return None,
            Intent::Space | Intent::Backspace => return None,
            Intent::Up
                if self.search.recalled.is_some()
                    || (empty
                        && self.page().is_none()
                        && self.search.selected == 0
                        && !(self.search.explicit
                            && self.command().query_eligibility.allows(&self.search.query))) =>
            {
                self.recall(true)
            }
            Intent::Down if self.search.recalled.is_some() => self.recall(false),
            Intent::Up | Intent::Down => self.move_selection(intent),
            Intent::Open | Intent::OpenAlternate => self.open(intent),
            Intent::Dismiss if self.page().is_some() => self.leave_view(),
            Intent::Dismiss => self.dismiss(),
        })
    }

    /// Types `keyword` and Space, as a row the keyword reaches offers: a command's keyword
    /// enters its search, a web search's alias is left in the query, and no keyword is Space on
    /// an empty query.
    pub(super) fn enter_keyword(&mut self, keyword: &str) -> Vec<Effect> {
        if !keyword.is_empty() {
            return self.complete(&format!("{keyword} "));
        }
        let Some(command) = self.commands.space_on_empty(&self.search.command).cloned() else {
            return Vec::new();
        };
        self.search.query.clear();
        let mut effects = vec![Effect::ClearQuery];
        effects.extend(self.switch(command, true));
        effects
    }

    fn complete(&mut self, completion: &str) -> Vec<Effect> {
        let mut effects = self.set_query(completion);
        if let Some(Effect::ReplaceQuery(text)) = effects
            .iter_mut()
            .find(|effect| matches!(effect, Effect::ReplaceQuery(_)))
        {
            *text = self.search.query.clone();
        } else {
            effects.insert(0, Effect::ReplaceQuery(self.search.query.clone()));
        }
        effects
    }

    pub(super) fn dismiss(&mut self) -> Vec<Effect> {
        let mut effects = vec![Effect::Hide];
        effects.extend(self.reset());
        effects
    }

    pub(super) fn remember_query(&mut self, activated: bool) {
        if !matches!(self.search.location, SearchLocation::Command) {
            return;
        }
        if activated {
            self.search.deleting = false;
            self.search.before_deletion = None;
        } else if let Some(query) = self.search.before_deletion.take() {
            self.record_query(query);
            return;
        }
        let query = &self.search.query;
        if (!activated && self.search.results != Results::Done) || query.trim().is_empty() {
            return;
        }
        self.record_query(query.to_owned());
    }

    fn record_query(&mut self, query: String) {
        if self.command().private_results {
            return;
        }
        let previous = self
            .queries
            .iter()
            .rev()
            .find(|(command, _)| *command == self.search.command);
        if previous.is_some_and(|(_, text)| *text == query) {
            return;
        }
        self.queries.push_back((self.search.command.clone(), query));
        if self.queries.len() > QUERY_HISTORY_LIMIT {
            self.queries.pop_front();
        }
    }

    fn recall(&mut self, backwards: bool) -> Vec<Effect> {
        let next = if backwards {
            let end = self.search.recalled.unwrap_or(self.queries.len());
            (0..end).rev().find(|&i| self.queries[i].0 == self.search.command)
        } else {
            (self.search.recalled.unwrap() + 1..self.queries.len())
                .find(|&i| self.queries[i].0 == self.search.command)
        };
        if backwards && next.is_none() {
            return Vec::new();
        }
        self.search.recalled = next;
        self.search.deleting = false;
        self.search.before_deletion = None;
        self.search.query = next.map_or_else(String::new, |i| self.queries[i].1.clone());
        let mut effects = vec![Effect::ReplaceQuery(self.search.query.clone())];
        effects.extend(self.search());
        effects
    }

    fn switch(&mut self, command: CommandId, explicit: bool) -> Vec<Effect> {
        self.search.recalled = None;
        self.search.deleting = false;
        self.search.before_deletion = None;
        self.search.command = command;
        self.search.explicit = explicit;
        self.search()
    }
}
