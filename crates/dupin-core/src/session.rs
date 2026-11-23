//! The launcher session: which registered command is active, the results it shows and which
//! one is selected. The UI forwards query edits and key intents; the session answers with
//! effects for the UI and platform layers to carry out.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock, RwLockReadGuard};

use crate::file_query::{FileKind, FileQuery};
use crate::item::Transfer;
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
    /// Select this byte range of the input, focusing it.
    SelectQuery(Range<usize>),
    /// Focus the input and select the part of the name the platform treats as its stem.
    SelectRenameStem(PathBuf),
    /// Rename the entry at `path` to `name` in the same folder. `row` is the layout key of the
    /// row showing it, which outlasts searches started while the rename runs.
    Rename {
        row: String,
        path: PathBuf,
        name: String,
    },
    Transfer {
        transfer: Transfer,
        path: PathBuf,
        folder: PathBuf,
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
pub struct Destination {
    path: PathBuf,
    transfer: Transfer,
    original: SearchState,
}

impl Destination {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn transfer(&self) -> Transfer {
        self.transfer
    }
}

#[derive(Debug)]
pub struct Session {
    commands: CommandRegistry,
    search: SearchState,
    last_generation: u64,
    history: Arc<RwLock<History>>,
    /// Unix seconds; a parameter so tests can fix the time.
    clock: fn() -> u64,
    queries: VecDeque<(CommandId, String)>,
    renaming: Option<Renaming>,
    destination: Option<Destination>,
}

#[derive(Debug)]
struct SearchState {
    generation: u64,
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
    results: Results,
    refresh_selection: Option<(CommandId, String)>,
    recalled: Option<usize>,
    deleting: bool,
    before_deletion: Option<String>,
}

/// Read access to the session's ranked history for the recent-items provider.
#[derive(Debug)]
pub struct RecentHistory {
    history: Arc<RwLock<History>>,
    clock: fn() -> u64,
}

impl RecentHistory {
    pub fn ranked_ids(&self) -> Vec<String> {
        self.history
            .read()
            .unwrap()
            .ranked_ids(usize::MAX, (self.clock)())
    }
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

impl SearchState {
    fn new(command: CommandId, explicit: bool) -> Self {
        Self {
            generation: 0,
            command,
            explicit,
            query: String::new(),
            items: Vec::new(),
            origins: Vec::new(),
            providers: BTreeMap::new(),
            active_order: Vec::new(),
            active_pin: None,
            active_limit: 0,
            selected: 0,
            selection_held: false,
            results: Results::Idle,
            refresh_selection: None,
            recalled: None,
            deleting: false,
            before_deletion: None,
        }
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

    fn remove_visible(&mut self, id: &str, origin_matches: impl Fn(&CommandId) -> bool) {
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
    pub fn new(history: History, commands: CommandRegistry) -> Result<Session, crate::CommandError> {
        commands.validate()?;
        let command = commands.default().clone();
        Ok(Session {
            commands,
            search: SearchState::new(command, false),
            last_generation: 0,
            history: Arc::new(RwLock::new(history)),
            clock: unix_now,
            queries: VecDeque::new(),
            renaming: None,
            destination: None,
        })
    }

    pub fn set_commands(&mut self, commands: CommandRegistry) -> Result<(), crate::CommandError> {
        commands.validate()?;
        for search in std::iter::once(&self.search)
            .chain(self.destination.iter().map(|destination| &destination.original))
        {
            if !commands.contains(&search.command) {
                return Err(crate::CommandError::ActiveCommandRemoved(search.command.clone()));
            }
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
        if self.destination.is_some() {
            return Err("Finish choosing a destination before saving.".into());
        }
        if self.search.results != Results::Done || self.search.items.is_empty() {
            return Err("Wait for the current search to finish before saving.".into());
        }
        let snapshot = crate::snapshots::Snapshot {
            saved_at: 0,
            version: 1,
            id: String::new(),
            mode: self.search.command.as_str().into(),
            explicit: self.search.explicit,
            query: self.search.query.clone(),
            title: self
                .selected_item()
                .map(|i| i.display_title())
                .unwrap_or(&self.search.query)
                .into(),
            views: self.search.items.iter().map(|i| i.view.clone()).collect(),
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
        self.destination = None;
        self.search.command = command;
        self.search.explicit = saved.explicit || self.search.command != *self.commands.default();
        self.search.query = saved.query.clone();
        self.search.recalled = None;
        self.search.deleting = false;
        self.search.before_deletion = None;
        let mut effects = vec![Effect::ReplaceQuery(self.search.query.clone())];
        effects.extend(self.search());
        Ok(effects)
    }

    pub fn history(&self) -> RwLockReadGuard<'_, History> {
        self.history.read().unwrap()
    }

    pub fn recent_history(&self) -> RecentHistory {
        RecentHistory {
            history: self.history.clone(),
            clock: self.clock,
        }
    }

    pub fn command_id(&self) -> &CommandId {
        &self.search.command
    }

    pub fn command_ids(&self) -> Vec<String> {
        self.commands.ids()
    }

    pub fn command(&self) -> &crate::CommandSpec {
        self.commands
            .command(&self.search.command)
            .expect("current command remains registered")
    }

    pub fn query(&self) -> &str {
        &self.search.query
    }

    pub fn items(&self) -> &[Item] {
        &self.search.items
    }

    pub fn selected(&self) -> usize {
        self.search.selected
    }

    pub fn selected_item(&self) -> Option<&Item> {
        self.search.items.get(self.search.selected)
    }

    pub fn renaming(&self) -> Option<&Renaming> {
        self.renaming.as_ref()
    }

    pub fn destination(&self) -> Option<&Destination> {
        self.destination.as_ref()
    }

    fn begin_destination(&mut self, path: PathBuf, transfer: Transfer) -> Vec<Effect> {
        let Some(command) = self.commands.destination_search().cloned() else {
            return Vec::new();
        };
        let original = std::mem::replace(&mut self.search, SearchState::new(command, true));
        self.destination = Some(Destination {
            path,
            transfer,
            original,
        });
        self.advance_generation();
        vec![
            Effect::CancelSearch,
            Effect::ClearQuery,
            Effect::SelectQuery(0..0),
        ]
    }

    fn finish_destination(&mut self, folder: Option<PathBuf>) -> Vec<Effect> {
        let Some(Destination {
            path,
            transfer,
            original,
        }) = self.destination.take()
        else {
            return Vec::new();
        };
        self.search = original;
        let mut effects = vec![
            Effect::CancelSearch,
            Effect::ReplaceQuery(self.search.query.clone()),
        ];
        // Resume an unfinished search without losing the rows or selection it had already found.
        if matches!(self.search.results, Results::Searching { .. }) {
            effects.extend(self.refresh());
        }
        effects.push(Effect::RevealSelection);
        if let Some(folder) = folder {
            effects.push(Effect::Transfer {
                transfer,
                path,
                folder,
            });
        }
        effects
    }

    fn begin_rename(&mut self, path: PathBuf) -> Vec<Effect> {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return Vec::new();
        };
        let name = name.to_owned();
        self.renaming = Some(Renaming {
            layout_key: self.layout_key(self.search.selected),
            path: path.clone(),
            name: name.clone(),
            query: self.search.query.clone(),
        });
        self.search.selection_held = true;
        self.search.refresh_selection = None;
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
            .get(self.search.selected)
            .filter(|item| !item.actions.is_empty())
            .map(|_| {
                vec![Effect::ShowActions {
                    id: self.result_key(self.search.selected),
                    generation: self.search.generation,
                }]
            })
            .unwrap_or_default();
        if !effects.is_empty() {
            self.search.selection_held = true;
            self.search.refresh_selection = None;
        }
        effects
    }

    fn actionable_items(&self) -> &[Item] {
        if matches!(self.search.results, Results::Searching { replaced: false }) {
            &[]
        } else {
            &self.search.items
        }
    }

    fn activate(&mut self, item: &Item, action: ItemAction) -> Vec<Effect> {
        if self.destination.is_some()
            || action.confirmation().is_some()
            || matches!(action.action, Action::ChooseApplication(_))
        {
            return Vec::new();
        }
        match action.action {
            Action::Rename(path) => return self.begin_rename(path),
            Action::ChooseDestination { path, transfer } => return self.begin_destination(path, transfer),
            _ => {}
        }
        if action.disposition == ActionDisposition::Consume {
            return vec![Effect::RunConsuming {
                id: self.result_key(self.search.selected),
                action: action.action,
            }];
        }
        let mut effects = vec![Effect::Run(action.action)];
        if action.disposition == ActionDisposition::Dismiss {
            if item.record_usage {
                self.history
                    .write()
                    .unwrap()
                    .record(&item.id, &self.search.query, (self.clock)());
                effects.push(Effect::SaveHistory);
            }
            self.remember_query(true);
            effects.extend(self.dismiss());
        }
        effects
    }

    pub fn results(&self) -> Results {
        self.search.results
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
            && self.search.command == *self.commands.default()
            && let Some((command, term)) = self.keyword_query(text)
        {
            self.search.deleting = false;
            self.search.before_deletion = None;
            self.search.items.clear();
            self.search.origins.clear();
            self.search.selected = 0;
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

    /// A key the UI recognized. None leaves the key to the text field (typing a space into a
    /// query, deleting a character); Some consumes it, even when nothing else happens.
    pub fn intent(&mut self, intent: Intent) -> Option<Vec<Effect>> {
        if self.renaming.is_some() {
            return self.rename_intent(intent);
        }
        if self.destination.is_some() {
            return match intent {
                Intent::Open => {
                    let effects = match self
                        .actionable_items()
                        .get(self.search.selected)
                        .and_then(|item| item.action("open"))
                    {
                        Some(Action::Open(folder)) => self.finish_destination(Some(folder.clone())),
                        _ => Vec::new(),
                    };
                    Some(effects)
                }
                Intent::Dismiss => Some(self.finish_destination(None)),
                Intent::Up | Intent::Down => Some(self.move_selection(intent)),
                Intent::Space | Intent::Backspace => None,
                Intent::Complete | Intent::Actions | Intent::OpenAlternate => Some(Vec::new()),
            };
        }
        let empty = self.search.query.is_empty();
        Some(match intent {
            Intent::Actions => self.actions(),
            Intent::Complete => {
                let completion = self
                    .actionable_items()
                    .get(self.search.selected)?
                    .completion
                    .clone()?;
                let mut effects = self.set_query(&completion);
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
            Intent::Backspace if empty && self.search.command != *self.commands.default() => {
                self.switch(self.commands.default().clone(), false)
            }
            Intent::Backspace if empty => return None,
            Intent::Space | Intent::Backspace => return None,
            Intent::Up
                if self.search.recalled.is_some()
                    || (empty
                        && self.search.selected == 0
                        && !(self.search.explicit
                            && self.command().query_eligibility.allows(&self.search.query))) =>
            {
                self.recall(true)
            }
            Intent::Down if self.search.recalled.is_some() => self.recall(false),
            Intent::Up | Intent::Down => self.move_selection(intent),
            Intent::Open | Intent::OpenAlternate => {
                let Some(item) = self.actionable_items().get(self.search.selected) else {
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

    fn move_selection(&mut self, intent: Intent) -> Vec<Effect> {
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

    pub fn provider_spec(&self, id: &CommandId) -> Option<&crate::CommandSpec> {
        self.commands.command(id)
    }
    pub(crate) fn provider_ids(&self) -> Vec<CommandId> {
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
    pub fn run_key_action(&mut self, key: &str, action_id: &str) -> Vec<Effect> {
        if !self.select_key(key) {
            return Vec::new();
        }
        let item = self.search.items[self.search.selected].clone();
        let Some(action) = item.item_action(action_id).cloned() else {
            return Vec::new();
        };
        self.activate(&item, action)
    }

    pub fn run_confirmed_key_action(&mut self, key: &str, action_id: &str) -> Vec<Effect> {
        if !self.select_key(key) {
            return Vec::new();
        }
        let item = self.search.items[self.search.selected].clone();
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
        let item = self.search.items[self.search.selected].clone();
        let Some(Action::ChooseApplication(parent)) = item.action(parent_id) else {
            return Vec::new();
        };
        if !matches!(&choice, Action::OpenWith { path, .. } if path == parent) {
            return Vec::new();
        }
        self.activate(&item, ItemAction::new(parent_id, "Open with", choice))
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

    #[cfg(test)]
    pub(crate) fn deliver(&mut self, delivery: Delivery) -> bool {
        self.deliver_from(self.search.command.clone(), delivery)
    }
    pub fn deliver_from(&mut self, origin: CommandId, delivery: Delivery) -> bool {
        if !self.pending(&origin, delivery.generation()) {
            return false;
        }
        let selected = if self.search.selection_held
            && matches!(self.search.results, Results::Searching { replaced: true })
        {
            self.search
                .items
                .get(self.search.selected)
                .map(|item| (self.search.origins[self.search.selected].clone(), item.id.clone()))
        } else {
            None
        };
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
        self.search.results = if pending {
            Results::Searching { replaced: true }
        } else {
            Results::Done
        };
        true
    }

    /// Back to an empty default-command search, as the panel should look the next time it shows.
    pub fn reset(&mut self) -> Vec<Effect> {
        let was_searching = matches!(self.search.results, Results::Searching { .. });
        if let Some(destination) = self.destination.take() {
            self.search = destination.original;
        }
        self.remember_query(false);
        self.renaming = None;
        self.search = SearchState::new(self.commands.default().clone(), false);
        let effects = self.search();
        if effects.is_empty() && was_searching {
            vec![Effect::CancelSearch]
        } else {
            effects
        }
    }

    fn dismiss(&mut self) -> Vec<Effect> {
        let mut effects = vec![Effect::Hide];
        effects.extend(self.reset());
        effects
    }

    fn remember_query(&mut self, activated: bool) {
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

    fn search(&mut self) -> Vec<Effect> {
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
            })
            .collect()
    }

    fn advance_generation(&mut self) {
        self.last_generation += 1;
        self.search.generation = self.last_generation;
    }

    fn clear_results(&mut self) {
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

#[cfg(test)]
mod tests;
