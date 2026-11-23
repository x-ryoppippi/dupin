//! The launcher session: which registered command is active, the results it shows and which
//! one is selected. The UI forwards query edits and key intents; the session answers with
//! effects for the UI and platform layers to carry out.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::searcher::Delivery;
use crate::{Action, ActionDisposition, CommandId, CommandRegistry, History, Item, ItemAction};

const QUERY_HISTORY_LIMIT: usize = 100;

/// Keys the session gives meaning to. The UI maps raw keys onto these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Space typed without modifiers.
    Space,
    Backspace,
    Up,
    Down,
    /// Complete the selected result into the query field.
    Complete,
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
        command: CommandId,
        generation: u64,
        query: String,
    },
    /// Stop whatever search is running; nothing should arrive for the current results.
    CancelSearch,
    Run(Action),
    RunConsuming {
        id: String,
        action: Action,
    },
    ShowActions {
        id: String,
        generation: u64,
    },
    /// Hide the panel. The session has already reset itself for the next show.
    Hide,
    /// The selection moved; keep the selected row on screen.
    RevealSelection,
    /// Empty the text field: a command keyword was consumed. The session's query is already empty.
    ClearQuery,
    /// Replace the input with the session's query after routing or recall.
    ReplaceQuery(String),
    /// Focus the input and select the part of the name the platform treats as its stem.
    SelectRenameStem(PathBuf),
    /// Rename the entry at `path` to `name` in the same folder. `row` is the layout key of the
    /// row showing it, which outlasts searches started while the rename runs.
    Rename {
        row: String,
        path: PathBuf,
        name: String,
    },
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

/// A rename in progress: the input holds the proposed name, not a query, until Enter
/// confirms or Esc cancels it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Renaming {
    layout_key: String,
    path: PathBuf,
    name: String,
    /// The search to put back afterwards.
    query: String,
}

impl Renaming {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn layout_key(&self) -> &str {
        &self.layout_key
    }

    /// Why Enter can't rename to the proposed name.
    pub fn problem(&self) -> Option<&'static str> {
        crate::file_name::problem(&self.name)
    }

    fn unchanged(&self) -> bool {
        self.path.file_name().is_some_and(|name| *name == *self.name)
    }
}

#[derive(Debug)]
pub struct Session {
    commands: CommandRegistry,
    command: CommandId,
    explicit: bool,
    query: String,
    items: Vec<Item>,
    origins: Vec<CommandId>,
    providers: BTreeMap<CommandId, ProviderState>,
    active_order: Vec<CommandId>,
    active_pin: Option<CommandId>,
    active_limit: usize,
    selected: usize,
    selection_held: bool,
    generation: u64,
    results: Results,
    refresh_selection: Option<(CommandId, String)>,
    history: History,
    /// Unix seconds; a parameter so tests can fix the time.
    clock: fn() -> u64,
    queries: VecDeque<(CommandId, String)>,
    recalled: Option<usize>,
    deleting: bool,
    before_deletion: Option<String>,
    renaming: Option<Renaming>,
}

#[derive(Debug)]
struct ProviderState {
    items: Vec<Item>,
    ids: HashSet<String>,
    phase: ProviderPhase,
    fallback: bool,
    candidate_limit: usize,
    result_limit: usize,
}

#[derive(Debug, PartialEq, Eq)]
enum ProviderPhase {
    Pending,
    Done,
    Failed,
    Expired,
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Session {
    pub fn new(history: History, commands: CommandRegistry) -> Result<Session, crate::CommandError> {
        commands.validate()?;
        let command = commands.default().clone();
        Ok(Session {
            commands,
            command,
            explicit: false,
            query: String::new(),
            items: Vec::new(),
            origins: Vec::new(),
            providers: BTreeMap::new(),
            active_order: Vec::new(),
            active_pin: None,
            active_limit: 0,
            selected: 0,
            selection_held: false,
            generation: 0,
            results: Results::Idle,
            refresh_selection: None,
            history,
            clock: unix_now,
            queries: VecDeque::new(),
            recalled: None,
            deleting: false,
            before_deletion: None,
            renaming: None,
        })
    }

    pub fn set_commands(&mut self, commands: CommandRegistry) -> Result<(), crate::CommandError> {
        commands.validate()?;
        if !commands.contains(&self.command) {
            return Err(crate::CommandError::ActiveCommandRemoved(self.command.clone()));
        }
        self.commands = commands;
        Ok(())
    }

    /// The same, with a clock in Unix seconds other than the system's.
    #[cfg(test)]
    fn with_clock(
        history: History,
        commands: CommandRegistry,
        clock: fn() -> u64,
    ) -> Result<Session, crate::CommandError> {
        Ok(Session {
            clock,
            ..Session::new(history, commands)?
        })
    }

    /// Capture only settled content belonging to the current search generation.
    pub fn capture(&self) -> Result<crate::snapshots::Snapshot, String> {
        if self.results != Results::Done || self.items.is_empty() {
            return Err("Wait for the current search to finish before saving.".into());
        }
        let snapshot = crate::snapshots::Snapshot {
            saved_at: 0,
            version: 1,
            id: String::new(),
            mode: self.command.as_str().into(),
            explicit: self.explicit,
            query: self.query.clone(),
            title: self
                .selected_item()
                .map(|i| i.display_title())
                .unwrap_or(&self.query)
                .into(),
            views: self.items.iter().map(|i| i.view.clone()).collect(),
            state: None,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    /// Explicitly rerun a saved search without activating a result or recording a pick.
    pub fn restore(&mut self, saved: &crate::snapshots::Snapshot) -> Result<Vec<Effect>, String> {
        saved.validate()?;
        let command = CommandId::new(saved.mode.clone()).map_err(|_| "Invalid saved mode.")?;
        if !self.commands.contains(&command) {
            return Err("The source command is unavailable. Saved content remains readable.".into());
        }
        self.clear_results();
        self.renaming = None;
        self.command = command;
        self.explicit = saved.explicit || self.command != *self.commands.default();
        self.query = saved.query.clone();
        self.recalled = None;
        self.deleting = false;
        self.before_deletion = None;
        let mut effects = vec![Effect::ReplaceQuery(self.query.clone())];
        effects.extend(self.search());
        Ok(effects)
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    pub fn command_id(&self) -> &CommandId {
        &self.command
    }

    pub fn command(&self) -> &crate::CommandSpec {
        self.commands
            .command(&self.command)
            .expect("current command remains registered")
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

    pub fn renaming(&self) -> Option<&Renaming> {
        self.renaming.as_ref()
    }

    fn begin_rename(&mut self, path: PathBuf) -> Vec<Effect> {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return Vec::new();
        };
        let name = name.to_owned();
        self.renaming = Some(Renaming {
            layout_key: self.layout_key(self.selected),
            path: path.clone(),
            name: name.clone(),
            query: self.query.clone(),
        });
        self.selection_held = true;
        self.refresh_selection = None;
        vec![Effect::ReplaceQuery(name), Effect::SelectRenameStem(path)]
    }

    fn rename_intent(&mut self, intent: Intent) -> Option<Vec<Effect>> {
        Some(match intent {
            Intent::Open => {
                let renaming = self.renaming.as_ref()?;
                if !renaming.unchanged() && renaming.problem().is_some() {
                    return Some(Vec::new());
                }
                let renaming = self.renaming.take()?;
                let mut effects = vec![Effect::ReplaceQuery(renaming.query.clone())];
                if !renaming.unchanged() {
                    effects.push(Effect::Rename {
                        row: renaming.layout_key,
                        path: renaming.path,
                        name: renaming.name,
                    });
                }
                effects
            }
            Intent::Dismiss => {
                let renaming = self.renaming.take()?;
                vec![Effect::ReplaceQuery(renaming.query)]
            }
            Intent::Space | Intent::Backspace => return None,
            Intent::Up | Intent::Down | Intent::Complete | Intent::OpenAlternate | Intent::Actions => {
                Vec::new()
            }
        })
    }

    fn actions(&mut self) -> Vec<Effect> {
        let effects = self
            .actionable_items()
            .get(self.selected)
            .filter(|item| !item.actions.is_empty())
            .map(|_| {
                vec![Effect::ShowActions {
                    id: self.result_key(self.selected),
                    generation: self.generation,
                }]
            })
            .unwrap_or_default();
        if !effects.is_empty() {
            self.selection_held = true;
            self.refresh_selection = None;
        }
        effects
    }

    fn actionable_items(&self) -> &[Item] {
        if matches!(self.results, Results::Searching { replaced: false }) {
            &[]
        } else {
            &self.items
        }
    }

    fn activate(&mut self, item: &Item, action: ItemAction) -> Vec<Effect> {
        if action.confirmation().is_some() || matches!(action.action, Action::ChooseApplication(_)) {
            return Vec::new();
        }
        if let Action::Rename(path) = action.action {
            return self.begin_rename(path);
        }
        if action.disposition == ActionDisposition::Consume {
            return vec![Effect::RunConsuming {
                id: self.result_key(self.selected),
                action: action.action,
            }];
        }
        let mut effects = vec![Effect::Run(action.action)];
        if action.disposition == ActionDisposition::Dismiss {
            if item.record_usage {
                self.history.record(&item.id, &self.query, (self.clock)());
                effects.push(Effect::SaveHistory);
            }
            self.remember_query(true);
            effects.extend(self.dismiss());
        }
        effects
    }

    pub fn results(&self) -> Results {
        self.results
    }

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
                self.before_deletion = (self.results == Results::Done).then(|| self.query.clone());
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
        if route_keyword
            && self.command == *self.commands.default()
            && let Some((command, term)) = self.keyword_query(text)
        {
            self.deleting = false;
            self.before_deletion = None;
            self.items.clear();
            self.selected = 0;
            self.query = term.to_owned();
            let mut effects = vec![Effect::ReplaceQuery(self.query.clone())];
            effects.extend(self.switch(command));
            return effects;
        }
        self.query = text.to_owned();
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

    /// Search the current query again after a provider's underlying data changes.
    pub fn refresh(&mut self) -> Vec<Effect> {
        let held = self.selection_held;
        let selected = self
            .selected_item()
            .filter(|_| held)
            .map(|item| (self.origins[self.selected].clone(), item.id.clone()));
        let effects = self.search();
        self.selection_held = held;
        self.refresh_selection = selected;
        effects
    }

    /// A key the UI recognized. None leaves the key to the text field (typing a space into a
    /// query, deleting a character); Some consumes it, even when nothing else happens.
    pub fn intent(&mut self, intent: Intent) -> Option<Vec<Effect>> {
        if self.renaming.is_some() {
            return self.rename_intent(intent);
        }
        let empty = self.query.is_empty();
        Some(match intent {
            Intent::Actions => self.actions(),
            Intent::Complete => {
                let completion = self.actionable_items().get(self.selected)?.completion.clone()?;
                let mut effects = self.set_query(&completion);
                if let Some(Effect::ReplaceQuery(text)) = effects
                    .iter_mut()
                    .find(|effect| matches!(effect, Effect::ReplaceQuery(_)))
                {
                    *text = self.query.clone();
                } else {
                    effects.insert(0, Effect::ReplaceQuery(self.query.clone()));
                }
                effects
            }
            Intent::Space if empty => self
                .commands
                .space_on_empty(&self.command)
                .cloned()
                .map_or_else(Vec::new, |command| self.switch(command)),
            Intent::Space if self.command == *self.commands.default() => {
                let command = self.commands.keyword(&self.query)?.clone();
                self.query.clear();
                let mut effects = vec![Effect::ClearQuery];
                effects.extend(self.switch(command));
                effects
            }
            Intent::Backspace if empty && self.command != *self.commands.default() => {
                let effects = self.switch(self.commands.default().clone());
                self.explicit = false;
                effects
            }
            Intent::Backspace if empty => return None,
            Intent::Space | Intent::Backspace => return None,
            Intent::Up if empty || self.recalled.is_some() => self.recall(true),
            Intent::Down if self.recalled.is_some() => self.recall(false),
            Intent::Up | Intent::Down => {
                self.refresh_selection = None;
                let n = self.items.len();
                if n == 0 {
                    return Some(Vec::new());
                }
                self.selection_held = !self.actionable_items().is_empty();
                self.selected = if intent == Intent::Down {
                    (self.selected + 1) % n
                } else {
                    (self.selected + n - 1) % n
                };
                vec![Effect::RevealSelection]
            }
            Intent::Open | Intent::OpenAlternate => {
                let Some(item) = self.actionable_items().get(self.selected) else {
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
                let Some(action) = item.item_action(id).cloned() else {
                    return Some(Vec::new());
                };
                let item = item.clone();
                self.activate(&item, action)
            }
            Intent::Dismiss => self.dismiss(),
        })
    }

    pub fn provider_spec(&self, id: &CommandId) -> Option<&crate::CommandSpec> {
        self.commands.command(id)
    }
    pub(crate) fn provider_ids(&self) -> Vec<CommandId> {
        if self.explicit {
            vec![self.command.clone()]
        } else {
            self.commands.providers(&self.command)
        }
    }
    pub fn failures(&self) -> impl Iterator<Item = (&CommandId, &'static str)> {
        self.providers.iter().filter_map(|(id, state)| match state.phase {
            ProviderPhase::Failed => Some((id, "Search unavailable")),
            ProviderPhase::Expired => Some((id, "Search took too long")),
            _ => None,
        })
    }
    pub fn pending(&self, origin: &CommandId, generation: u64) -> bool {
        generation == self.generation
            && self
                .providers
                .get(origin)
                .is_some_and(|s| s.phase == ProviderPhase::Pending)
    }
    pub fn layout_key(&self, index: usize) -> String {
        format!("{}:{}", self.origins[index], self.items[index].id)
    }
    pub fn result_key(&self, index: usize) -> String {
        format!(
            "{}:{}:{}",
            self.generation, self.origins[index], self.items[index].id
        )
    }
    pub fn select_key(&mut self, key: &str) -> bool {
        if self.renaming.is_some() || matches!(self.results, Results::Searching { replaced: false }) {
            return false;
        }
        if let Some(index) = (0..self.items.len()).find(|&i| self.result_key(i) == key) {
            self.selected = index;
            self.selection_held = true;
            self.refresh_selection = None;
            true
        } else {
            false
        }
    }
    pub fn run_key_action(&mut self, key: &str, action_id: &str) -> Vec<Effect> {
        if !self.select_key(key) {
            return Vec::new();
        }
        let item = self.items[self.selected].clone();
        let Some(action) = item.item_action(action_id).cloned() else {
            return Vec::new();
        };
        self.activate(&item, action)
    }

    pub fn run_confirmed_key_action(&mut self, key: &str, action_id: &str) -> Vec<Effect> {
        if !self.select_key(key) {
            return Vec::new();
        }
        let item = self.items[self.selected].clone();
        let Some(mut action) = item.item_action(action_id).cloned() else {
            return Vec::new();
        };
        if action.confirmation().is_none() || matches!(action.action, Action::ChooseApplication(_)) {
            return Vec::new();
        }
        action.confirmation = None;
        self.activate(&item, action)
    }

    pub fn run_key_choice(&mut self, key: &str, parent_id: &str, choice: Action) -> Vec<Effect> {
        if !self.select_key(key) {
            return Vec::new();
        }
        let item = self.items[self.selected].clone();
        let Some(Action::ChooseApplication(parent)) = item.action(parent_id) else {
            return Vec::new();
        };
        if !matches!(&choice, Action::OpenWith { path, .. } if path == parent) {
            return Vec::new();
        }
        self.activate(&item, ItemAction::new(parent_id, "Open with", choice))
    }

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
    pub fn replace(&mut self, row: &str, mut item: Item) -> bool {
        let Some((origin, id)) = self.locate(|origin, item| format!("{origin}:{}", item.id) == row) else {
            return false;
        };
        let provider = self.providers.get_mut(&origin).expect("located provider");
        if provider.items.iter().any(|row| row.id == item.id) {
            self.remove(&origin, &id);
            return true;
        }
        let index = provider
            .items
            .iter()
            .position(|row| row.id == id)
            .expect("located item");
        item.score = provider.items[index].score;
        // The old ID stays known, so a late batch can't bring back the entry's previous name.
        provider.ids.insert(item.id.clone());
        provider.items[index] = item.clone();
        if let Some(index) = (0..self.items.len()).find(|&i| self.layout_key(i) == row) {
            self.items[index] = item;
        }
        true
    }

    pub fn forget(&mut self, key: &str) -> bool {
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
        if let Some(index) =
            (0..self.items.len()).find(|&i| self.origins[i] == *origin && self.items[i].id == id)
        {
            self.items.remove(index);
            self.origins.remove(index);
            if index < self.selected {
                self.selected -= 1;
            }
            self.selected = self.selected.min(self.items.len().saturating_sub(1));
        }
    }
    #[cfg(test)]
    pub(crate) fn deliver(&mut self, delivery: Delivery) -> bool {
        self.deliver_from(self.command.clone(), delivery)
    }
    pub fn deliver_from(&mut self, origin: CommandId, delivery: Delivery) -> bool {
        if !self.pending(&origin, delivery.generation()) {
            return false;
        }
        let selected = if self.selection_held && matches!(self.results, Results::Searching { replaced: true })
        {
            self.items
                .get(self.selected)
                .map(|item| (self.origins[self.selected].clone(), item.id.clone()))
        } else {
            None
        };
        let state = self.providers.get_mut(&origin).unwrap();
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
                self.history.rerank(&mut state.items, &self.query, (self.clock)());
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
        let pending = self.providers.values().any(|s| s.phase == ProviderPhase::Pending);
        let show_fallback = self
            .providers
            .values()
            .filter(|state| !state.fallback)
            .all(|state| state.phase != ProviderPhase::Pending && state.items.is_empty());
        let mut items = Vec::new();
        let mut origins = Vec::new();
        let mut order = self.active_order.clone();
        if let Some(pin) = &self.active_pin {
            order.sort_by_key(|id| id != pin);
        }
        for id in order {
            let state = &self.providers[&id];
            if state.fallback && !show_fallback {
                continue;
            }
            let count = if self.active_pin.as_ref() == Some(&id) {
                state.result_limit.min(1)
            } else {
                state.result_limit
            };
            for item in state.items.iter().take(count) {
                if items.len() >= self.active_limit {
                    break;
                }
                items.push(item.clone());
                origins.push(id.clone());
            }
        }
        if items.is_empty() && pending && matches!(self.results, Results::Searching { replaced: false }) {
            let previous_items = std::mem::take(&mut self.items);
            let previous_origins = std::mem::take(&mut self.origins);
            let previous_selection = self.selected;
            self.selected = 0;
            for (index, (item, origin)) in previous_items.into_iter().zip(previous_origins).enumerate() {
                if self
                    .providers
                    .get(&origin)
                    .is_some_and(|state| state.phase == ProviderPhase::Pending)
                {
                    if index == previous_selection {
                        self.selected = self.items.len();
                    }
                    self.items.push(item);
                    self.origins.push(origin);
                }
            }
            return true;
        }
        self.items = items;
        self.origins = origins;
        let selection = self.refresh_selection.clone().or(selected);
        // A chosen result must remain actionable when newer candidates fill the visible slots.
        if self.selection_held
            && let Some((origin, id)) = &selection
            && !self
                .items
                .iter()
                .enumerate()
                .any(|(i, item)| item.id == *id && self.origins[i] == *origin)
            && let Some(state) = self.providers.get(origin)
            && (!state.fallback || show_fallback)
            && let Some(item) = state.items.iter().find(|item| item.id == *id).cloned()
        {
            let provider_limit = if self.active_pin.as_ref() == Some(origin) {
                state.result_limit.min(1)
            } else {
                state.result_limit
            };
            let full_provider = self.origins.iter().filter(|id| *id == origin).count() >= provider_limit;
            let slot = (0..self.items.len()).rev().find(|&i| {
                if full_provider {
                    self.origins[i] == *origin
                } else {
                    self.origins[i] == *origin || self.active_pin.as_ref() != Some(&self.origins[i])
                }
            });
            if let Some(slot) = slot {
                self.items[slot] = item;
                self.origins[slot] = origin.clone();
            }
        }
        self.selected = 0;
        if let Some((origin, id)) = selection
            && let Some(i) = self
                .items
                .iter()
                .enumerate()
                .position(|(i, item)| item.id == id && self.origins[i] == origin)
        {
            self.selected = i;
        }
        self.results = if pending {
            Results::Searching { replaced: true }
        } else {
            Results::Done
        };
        true
    }

    /// Back to an empty default-command search, as the panel should look the next time it shows.
    pub fn reset(&mut self) -> Vec<Effect> {
        self.remember_query(false);
        self.renaming = None;
        self.recalled = None;
        self.deleting = false;
        self.before_deletion = None;
        let was_searching = matches!(self.results, Results::Searching { .. });
        self.command = self.commands.default().clone();
        self.explicit = false;
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
        let query = &self.query;
        if (!activated && self.results != Results::Done) || query.trim().is_empty() {
            return;
        }
        self.record_query(query.to_owned());
    }

    fn record_query(&mut self, query: String) {
        let previous = self
            .queries
            .iter()
            .rev()
            .find(|(command, _)| *command == self.command);
        if previous.is_some_and(|(_, text)| *text == query) {
            return;
        }
        self.queries.push_back((self.command.clone(), query));
        if self.queries.len() > QUERY_HISTORY_LIMIT {
            self.queries.pop_front();
        }
    }

    fn recall(&mut self, backwards: bool) -> Vec<Effect> {
        let next = if backwards {
            let end = self.recalled.unwrap_or(self.queries.len());
            (0..end).rev().find(|&i| self.queries[i].0 == self.command)
        } else {
            (self.recalled.unwrap() + 1..self.queries.len()).find(|&i| self.queries[i].0 == self.command)
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

    fn switch(&mut self, command: CommandId) -> Vec<Effect> {
        self.recalled = None;
        self.deleting = false;
        self.before_deletion = None;
        self.command = command;
        self.explicit = true;
        self.search()
    }

    fn search(&mut self) -> Vec<Effect> {
        self.refresh_selection = None;
        self.selection_held = false;
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
        let query = self.query.clone();
        self.generation += 1;
        // A command change clears the previous command's rows before starting a search.
        self.results = Results::Searching { replaced: false };
        self.active_order = self.provider_ids();
        self.active_pin = self.commands.pinned().filter(|_| !self.explicit).cloned();
        self.active_limit = self.command().result_limit;
        self.providers = self
            .provider_ids()
            .into_iter()
            .map(|id| {
                (
                    id.clone(),
                    ProviderState {
                        items: Vec::new(),
                        ids: HashSet::new(),
                        phase: ProviderPhase::Pending,
                        fallback: !self.explicit && self.commands.command(&id).unwrap().is_fallback(),
                        candidate_limit: self.commands.command(&id).unwrap().candidate_limit(),
                        result_limit: self.commands.command(&id).unwrap().result_limit,
                    },
                )
            })
            .collect();
        self.provider_ids()
            .into_iter()
            .map(|command| Effect::Search {
                command,
                generation: self.generation,
                query: query.clone(),
            })
            .collect()
    }

    fn clear_results(&mut self) {
        self.selection_held = false;
        // Bump the generation so answers still in flight are recognized as stale.
        self.generation += 1;
        self.items.clear();
        self.origins.clear();
        self.providers.clear();
        self.selected = 0;
        self.results = Results::Idle;
    }
}

#[cfg(test)]
mod tests;
