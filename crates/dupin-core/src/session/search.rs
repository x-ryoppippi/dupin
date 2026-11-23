use std::collections::HashSet;
use std::path::Path;

use crate::file_query::{FileKind, FileQuery};
use crate::searcher::Delivery;
use crate::{CommandId, Item};

use super::{Effect, Results, SearchState, Session};

#[derive(Debug)]
pub(super) struct ProviderState {
    pub(super) items: Vec<Item>,
    ids: HashSet<String>,
    pub(super) phase: ProviderPhase,
    pub(super) fallback: bool,
    candidate_limit: usize,
    pub(super) result_limit: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ProviderPhase {
    Pending,
    Done,
    Failed,
    Expired,
}

impl SearchState {
    /// The provider and item ID behind a key of the current candidates, shown or not.
    fn locate(&self, matches: impl Fn(&CommandId, &Item) -> bool) -> Option<(CommandId, String)> {
        self.providers.iter().find_map(|(origin, state)| {
            state
                .items
                .iter()
                .find(|item| matches(origin, item))
                .map(|item| (origin.clone(), item.id.clone()))
        })
    }

    /// Put `item` where the row with layout key `row` was, as after the entry it showed was
    /// renamed. The row keeps its place and rank; when `item` is already a candidate, the old
    /// row just goes.
    fn replace(&mut self, row: &str, mut item: Item) -> bool {
        let matches = |origin: &CommandId, item: &Item| format!("{origin}:{}", item.id) == row;
        let visible = self
            .origins
            .iter()
            .zip(&self.items)
            .position(|(origin, item)| matches(origin, item));
        let Some((origin, id)) = visible
            .map(|index| (self.origins[index].clone(), self.items[index].id.clone()))
            .or_else(|| self.locate(matches))
        else {
            return false;
        };

        if let Some((selected_origin, selected_id)) = &mut self.refresh_selection
            && *selected_origin == origin
            && *selected_id == id
        {
            selected_id.clone_from(&item.id);
        }

        if let Some(index) = visible {
            item.score = self.items[index].score;
        }
        if let Some(provider) = self.providers.get_mut(&origin) {
            // A refresh may have cleared the candidates while keeping the old rows on screen.
            provider.ids.insert(id.clone());
            if id != item.id && provider.items.iter().any(|row| row.id == item.id) {
                self.remove(&origin, &id);
                return true;
            }
            if let Some(candidate) = provider.items.iter_mut().find(|row| row.id == id) {
                item.score = candidate.score;
                provider.ids.insert(item.id.clone());
                *candidate = item.clone();
            }
        }
        if let Some(index) = visible {
            self.items[index] = item;
        }
        true
    }

    fn forget(&mut self, key: &str) -> bool {
        let generation = self.generation;
        let Some((origin, id)) =
            self.locate(|origin, item| format!("{generation}:{origin}:{}", item.id) == key)
        else {
            return false;
        };
        self.remove(&origin, &id);
        true
    }

    fn remove(&mut self, origin: &CommandId, id: &str) {
        if let Some(provider) = self.providers.get_mut(origin) {
            provider.items.retain(|row| row.id != id);
        }
        self.remove_visible(id, |id| id == origin);
    }

    fn forget_path(&mut self, id: &str) {
        for provider in self.providers.values_mut() {
            provider.items.retain(|row| row.id != id);
            // A batch already on its way may still contain the old path.
            provider.ids.insert(id.to_owned());
        }
        self.remove_visible(id, |_| true);
    }
}

impl Session {
    pub(crate) fn provider_ids(&self) -> Vec<CommandId> {
        if self.page().is_some() {
            return vec![self.search.command.clone()];
        }
        let candidates = if self.search.explicit {
            vec![self.search.command.clone()]
        } else {
            self.commands.providers(&self.search.command)
        };
        candidates
            .into_iter()
            .filter(|id| {
                self.commands
                    .command(id)
                    .is_some_and(|spec| spec.query_eligibility.allows(&self.search.query))
            })
            .collect()
    }

    pub fn failures(&self) -> impl Iterator<Item = (&CommandId, &'static str)> {
        self.search
            .providers
            .iter()
            .filter_map(|(id, state)| match state.phase {
                ProviderPhase::Failed => Some((id, "Search unavailable")),
                ProviderPhase::Expired => Some((id, "Search took too long")),
                _ => None,
            })
    }

    pub fn pending(&self, origin: &CommandId, generation: u64) -> bool {
        generation == self.search.generation
            && self
                .search
                .providers
                .get(origin)
                .is_some_and(|s| s.phase == ProviderPhase::Pending)
    }

    /// Update the search where a rename completed, including one covered by the destination picker.
    pub fn replace(&mut self, row: &str, item: Item) -> bool {
        let search = self
            .destination
            .as_mut()
            .map_or(&mut self.search, |destination| &mut destination.original);
        search.replace(row, item)
    }

    pub fn forget(&mut self, key: &str) -> bool {
        let search = self
            .destination
            .as_mut()
            .map_or(&mut self.search, |destination| &mut destination.original);
        search.forget(key)
    }

    /// Remove a moved path even if the search changed while the operation ran.
    pub fn forget_path(&mut self, path: &Path) {
        let id = path.to_string_lossy();
        self.search.forget_path(&id);
        if let Some(destination) = &mut self.destination {
            destination.original.forget_path(&id);
        }
    }

    pub fn deliver_from(&mut self, origin: CommandId, delivery: Delivery) -> bool {
        if !self.pending(&origin, delivery.generation()) {
            return false;
        }
        let selected = self.held_selection();
        let state = self.search.providers.get_mut(&origin).unwrap();
        let limit = state.candidate_limit;
        match delivery {
            Delivery::Batch { items, .. } => {
                for item in items {
                    if state.items.len() >= limit {
                        break;
                    }
                    if state.ids.insert(item.id.clone()) {
                        state.items.push(item);
                    }
                }
                self.history
                    .read()
                    .unwrap()
                    .rerank(&mut state.items, &self.search.query, (self.clock)());
            }
            Delivery::Done { .. } => state.phase = ProviderPhase::Done,
            Delivery::Failed { .. } => {
                state.phase = ProviderPhase::Failed;
                state.items.clear();
                state.ids.clear();
            }
            Delivery::Expired { .. } => {
                state.phase = ProviderPhase::Expired;
            }
        }
        let pending = self
            .search
            .providers
            .values()
            .any(|s| s.phase == ProviderPhase::Pending);
        let show_fallback = self
            .search
            .providers
            .values()
            .filter(|state| !state.fallback)
            .all(|state| state.phase != ProviderPhase::Pending && state.items.is_empty());
        let mut items = Vec::new();
        let mut origins = Vec::new();
        let mut order = self.search.active_order.clone();
        if let Some(pin) = &self.search.active_pin {
            order.sort_by_key(|id| id != pin);
        }
        for id in order {
            let state = &self.search.providers[&id];
            if state.fallback && !show_fallback {
                continue;
            }
            let count = if self.search.active_pin.as_ref() == Some(&id) {
                state.result_limit.min(1)
            } else {
                state.result_limit
            };
            for item in state.items.iter().take(count) {
                if items.len() >= self.search.active_limit {
                    break;
                }
                items.push(item.clone());
                origins.push(id.clone());
            }
        }
        if items.is_empty()
            && pending
            && matches!(self.search.results, Results::Searching { replaced: false })
        {
            let previous_items = std::mem::take(&mut self.search.items);
            let previous_origins = std::mem::take(&mut self.search.origins);
            let previous_selection = self.search.selected;
            self.search.selected = 0;
            for (index, (item, origin)) in previous_items.into_iter().zip(previous_origins).enumerate() {
                if self
                    .search
                    .providers
                    .get(&origin)
                    .is_some_and(|state| state.phase == ProviderPhase::Pending)
                {
                    if index == previous_selection {
                        self.search.selected = self.search.items.len();
                    }
                    self.search.items.push(item);
                    self.search.origins.push(origin);
                }
            }
            return true;
        }
        self.search.items = items;
        self.search.origins = origins;
        self.restore_selection(selected, show_fallback);
        self.search.results = if pending {
            Results::Searching { replaced: true }
        } else {
            Results::Done
        };
        true
    }

    pub(super) fn search(&mut self) -> Vec<Effect> {
        self.search.refresh_selection = None;
        self.search.selection_held = false;
        let provider_ids = self.provider_ids();
        if provider_ids.is_empty() {
            let was_searching = matches!(self.search.results, Results::Searching { .. });
            self.clear_results();
            return if was_searching {
                vec![Effect::CancelSearch]
            } else {
                Vec::new()
            };
        }
        let query = if self.destination.is_some()
            && !FileQuery::parse(self.search.query.trim())
                .is_ok_and(|parsed| parsed.kind == Some(FileKind::Folder))
        {
            format!("kind:folder {}", self.search.query)
        } else {
            self.search.query.clone()
        };
        self.advance_generation();
        if self
            .search
            .origins
            .iter()
            .all(|origin| !provider_ids.contains(origin))
        {
            self.search.items.clear();
            self.search.origins.clear();
            self.search.selected = 0;
        }
        self.search.results = Results::Searching { replaced: false };
        self.search.active_order.clone_from(&provider_ids);
        self.search.active_pin = self
            .commands
            .pinned()
            .filter(|pin| !self.search.explicit && provider_ids.contains(pin))
            .cloned();
        self.search.active_limit = self.command().result_limit;
        self.search.providers = self
            .search
            .active_order
            .iter()
            .map(|id| {
                (
                    id.clone(),
                    ProviderState {
                        items: Vec::new(),
                        ids: HashSet::new(),
                        phase: ProviderPhase::Pending,
                        fallback: !self.search.explicit && self.commands.command(id).unwrap().is_fallback(),
                        candidate_limit: self.commands.command(id).unwrap().candidate_limit(),
                        result_limit: self.commands.command(id).unwrap().result_limit,
                    },
                )
            })
            .collect();
        provider_ids
            .into_iter()
            .map(|command| Effect::Search {
                command,
                generation: self.search.generation,
                query: query.clone(),
                folder: self.folder().map(Path::to_path_buf),
                page: self.page().cloned(),
            })
            .collect()
    }

    pub(super) fn advance_generation(&mut self) {
        self.last_generation += 1;
        self.search.generation = self.last_generation;
    }

    pub(super) fn clear_results(&mut self) {
        self.search.selection_held = false;
        // Bump the generation so answers still in flight are recognized as stale.
        self.advance_generation();
        self.search.items.clear();
        self.search.origins.clear();
        self.search.providers.clear();
        self.search.selected = 0;
        self.search.results = Results::Idle;
    }
}
