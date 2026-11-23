use crate::{CommandId, Item};

use super::{Effect, Intent, Results, SearchState, Session};

impl SearchState {
    pub(super) fn remove_visible(&mut self, id: &str, origin_matches: impl Fn(&CommandId) -> bool) {
        for index in (0..self.items.len()).rev() {
            if self.items[index].id == id && origin_matches(&self.origins[index]) {
                self.items.remove(index);
                self.origins.remove(index);
                if index < self.selected {
                    self.selected -= 1;
                }
            }
        }
        self.selected = self.selected.min(self.items.len().saturating_sub(1));
    }
}

impl Session {
    /// Search the current query again after a provider's underlying data changes.
    pub fn refresh(&mut self) -> Vec<Effect> {
        let held = self.search.selection_held;
        let selected = self
            .selected_item()
            .filter(|_| held)
            .map(|item| (self.search.origins[self.search.selected].clone(), item.id.clone()));
        let effects = self.search();
        self.search.selection_held = held;
        self.search.refresh_selection = selected;
        effects
    }

    pub(super) fn move_selection(&mut self, intent: Intent) -> Vec<Effect> {
        self.search.refresh_selection = None;
        let n = self.search.items.len();
        if n == 0 {
            return Vec::new();
        }
        self.search.selection_held = !self.actionable_items().is_empty();
        self.search.selected = if intent == Intent::Down {
            (self.search.selected + 1) % n
        } else {
            (self.search.selected + n - 1) % n
        };
        vec![Effect::RevealSelection]
    }

    pub fn layout_key(&self, index: usize) -> String {
        format!("{}:{}", self.search.origins[index], self.search.items[index].id)
    }

    pub fn result_key(&self, index: usize) -> String {
        format!(
            "{}:{}:{}",
            self.search.generation, self.search.origins[index], self.search.items[index].id
        )
    }

    pub fn select_key(&mut self, key: &str) -> bool {
        if self.renaming.is_some() || matches!(self.search.results, Results::Searching { replaced: false }) {
            return false;
        }
        if let Some(index) = (0..self.search.items.len()).find(|&i| self.result_key(i) == key) {
            self.search.selected = index;
            self.search.selection_held = true;
            self.search.refresh_selection = None;
            true
        } else {
            false
        }
    }

    pub(super) fn actionable_items(&self) -> &[Item] {
        if matches!(self.search.results, Results::Searching { replaced: false }) {
            &[]
        } else {
            &self.search.items
        }
    }

    pub(super) fn held_selection(&self) -> Option<(CommandId, String)> {
        if self.search.selection_held && matches!(self.search.results, Results::Searching { replaced: true })
        {
            self.search
                .items
                .get(self.search.selected)
                .map(|item| (self.search.origins[self.search.selected].clone(), item.id.clone()))
        } else {
            None
        }
    }

    pub(super) fn restore_selection(&mut self, selected: Option<(CommandId, String)>, show_fallback: bool) {
        let selection = self.search.refresh_selection.clone().or(selected);
        // A chosen result must remain actionable when newer candidates fill the visible slots.
        if self.search.selection_held
            && let Some((origin, id)) = &selection
            && !self
                .search
                .items
                .iter()
                .enumerate()
                .any(|(i, item)| item.id == *id && self.search.origins[i] == *origin)
            && let Some(state) = self.search.providers.get(origin)
            && (!state.fallback || show_fallback)
            && let Some(item) = state.items.iter().find(|item| item.id == *id).cloned()
        {
            let provider_limit = if self.search.active_pin.as_ref() == Some(origin) {
                state.result_limit.min(1)
            } else {
                state.result_limit
            };
            let full_provider =
                self.search.origins.iter().filter(|id| *id == origin).count() >= provider_limit;
            let slot = (0..self.search.items.len()).rev().find(|&i| {
                if full_provider {
                    self.search.origins[i] == *origin
                } else {
                    self.search.origins[i] == *origin
                        || self.search.active_pin.as_ref() != Some(&self.search.origins[i])
                }
            });
            if let Some(slot) = slot {
                self.search.items[slot] = item;
                self.search.origins[slot] = origin.clone();
            }
        }
        self.search.selected = 0;
        if let Some((origin, id)) = selection
            && let Some(i) = self
                .search
                .items
                .iter()
                .enumerate()
                .position(|(i, item)| item.id == id && self.search.origins[i] == origin)
        {
            self.search.selected = i;
        }
    }
}
