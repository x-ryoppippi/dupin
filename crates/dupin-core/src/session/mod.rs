//! The launcher session: which registered command is active, the results it shows and which
//! one is selected. The UI forwards query edits and key intents; the session answers with
//! effects for the UI and platform layers to carry out.

mod actions;
mod intents;
mod keywords;
mod navigation;
mod search;
mod selection;
mod snapshots;

pub use keywords::keyword_tag;
pub use navigation::{Anchor, Navigated};

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock, RwLockReadGuard};

use crate::item::Transfer;
use crate::{Action, CommandId, CommandRegistry, Detail, History, Item, Page};

use self::search::{Presentation, ProviderState};

const QUERY_HISTORY_LIMIT: usize = 100;
const MAX_PAGE_DEPTH: usize = 8;

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
        folder: Option<PathBuf>,
        /// The pushed view to search, if any.
        page: Option<Page>,
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
        confirmation: Option<String>,
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

#[derive(Debug)]
pub struct Destination {
    path: PathBuf,
    transfer: Transfer,
    original: SearchState,
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
    visits: Vec<Visit>,
    /// Counts every move between views, so an action's effect lands only on the view it was
    /// chosen on.
    navigation: u64,
}

#[derive(Debug)]
enum SearchLocation {
    Command,
    Folder(PathBuf),
    Page(Page),
}

#[derive(Debug)]
struct Visit {
    location: SearchLocation,
    command: CommandId,
    explicit: bool,
    query: String,
    selected: Option<(CommandId, String)>,
}

#[derive(Debug)]
struct SearchState {
    generation: u64,
    command: CommandId,
    explicit: bool,
    query: String,
    location: SearchLocation,
    items: Vec<Item>,
    origins: Vec<CommandId>,
    /// Sections and details of rows, by origin and item ID. Kept apart from the rows so
    /// they follow a row through ranking and survive while a search replaces it.
    presentations: HashMap<CommandId, HashMap<String, Presentation>>,
    providers: BTreeMap<CommandId, ProviderState>,
    active_order: Vec<CommandId>,
    active_pin: Option<CommandId>,
    active_limit: usize,
    /// The rows for the commands the main search's query is a keyword of, and how many of
    /// them lead the results; the rest follow the first of the providers' rows.
    suggestions: Vec<Item>,
    leading: usize,
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

/// The result chosen for an action that needs more input.
pub struct FollowupTarget {
    item: Item,
    command: CommandId,
    query: String,
}

pub fn unix_now() -> u64 {
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
            location: SearchLocation::Command,
            items: Vec::new(),
            origins: Vec::new(),
            presentations: HashMap::new(),
            providers: BTreeMap::new(),
            active_order: Vec::new(),
            active_pin: None,
            active_limit: 0,
            suggestions: Vec::new(),
            leading: 0,
            selected: 0,
            selection_held: false,
            results: Results::Idle,
            refresh_selection: None,
            recalled: None,
            deleting: false,
            before_deletion: None,
        }
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
            visits: Vec::new(),
            navigation: 0,
        })
    }

    /// Replace the command registry, returning effects if the active command disappears.
    pub fn set_commands(&mut self, commands: CommandRegistry) -> Result<Vec<Effect>, crate::CommandError> {
        commands.validate()?;
        if commands.contains(&self.search.command)
            && self.visits.iter().all(|visit| commands.contains(&visit.command))
            && self
                .destination
                .as_ref()
                .is_none_or(|destination| commands.contains(&destination.original.command))
        {
            let previous = self.provider_ids();
            self.commands = commands;
            return Ok(if self.provider_ids() == previous {
                Vec::new()
            } else {
                self.refresh()
            });
        }
        let mut effects = vec![Effect::ClearQuery];
        effects.extend(self.reset_to_default(Some(commands)));
        Ok(effects)
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

    /// The folder being browsed, if any.
    pub fn folder(&self) -> Option<&Path> {
        match &self.search.location {
            SearchLocation::Folder(folder) => Some(folder),
            _ => None,
        }
    }

    /// The top view, if one has been opened from a result.
    pub fn page(&self) -> Option<&Page> {
        match &self.search.location {
            SearchLocation::Page(page) => Some(page),
            _ => None,
        }
    }

    pub fn items(&self) -> &[Item] {
        &self.search.items
    }

    /// The provider each of [`Self::items`] came from, index for index.
    pub fn origins(&self) -> &[CommandId] {
        &self.search.origins
    }

    pub fn selected(&self) -> usize {
        self.search.selected
    }

    pub fn selected_item(&self) -> Option<&Item> {
        self.search.items.get(self.search.selected)
    }

    fn presentation(&self, index: usize) -> Option<&Presentation> {
        let origin = self.search.origins.get(index)?;
        let item = self.search.items.get(index)?;
        self.search.presentations.get(origin)?.get(&item.id)
    }

    fn section(&self, index: usize) -> Option<&str> {
        self.presentation(index)?.section.as_deref()
    }

    /// The heading to show above row `index`: its section, where that differs from the row
    /// above.
    pub fn section_start(&self, index: usize) -> Option<&str> {
        let section = self.section(index)?;
        (index == 0 || self.section(index - 1) != Some(section)).then_some(section)
    }

    pub fn detail(&self, index: usize) -> Option<&Detail> {
        let presentation = self.presentation(index)?;
        if presentation.detail_when_selected && index != self.search.selected {
            return None;
        }
        presentation.detail.as_deref()
    }

    /// What row `index` shows at its end.
    pub fn accessories(&self, index: usize) -> &[crate::Accessory] {
        self.presentation(index)
            .map_or(&[], |presentation| &presentation.accessories)
    }

    /// The form the page shows in place of a list, once its view answered with one.
    pub fn form(&self) -> Option<&Arc<crate::form::Form>> {
        self.page()?;
        self.presentation(0)?.form.as_ref()
    }

    /// What the search said to show while it has no rows.
    pub fn empty_text(&self) -> Option<&str> {
        if !self.search.items.is_empty() {
            return None;
        }
        self.search
            .providers
            .values()
            .find_map(|provider| provider.empty_text.as_deref())
    }

    pub fn renaming(&self) -> Option<&Renaming> {
        self.renaming.as_ref()
    }

    pub fn destination(&self) -> Option<&Destination> {
        self.destination.as_ref()
    }

    pub fn results(&self) -> Results {
        self.search.results
    }

    pub fn provider_spec(&self, id: &CommandId) -> Option<&crate::CommandSpec> {
        self.commands.command(id)
    }

    #[cfg(test)]
    fn deliver(&mut self, delivery: crate::searcher::Delivery) -> bool {
        self.deliver_from(self.search.command.clone(), delivery)
    }

    /// Back to an empty default-command search, as the panel should look the next time it shows.
    pub fn reset(&mut self) -> Vec<Effect> {
        self.reset_to_default(None)
    }

    fn reset_to_default(&mut self, commands: Option<CommandRegistry>) -> Vec<Effect> {
        let was_searching = matches!(self.search.results, Results::Searching { .. });
        if let Some(destination) = self.destination.take() {
            self.search = destination.original;
        }
        self.remember_query(false);
        self.visits.clear();
        self.navigation += 1;
        if let Some(commands) = commands {
            self.commands = commands;
        }
        self.renaming = None;
        self.search = SearchState::new(self.commands.default().clone(), false);
        let effects = self.search();
        if effects.is_empty() && was_searching {
            vec![Effect::CancelSearch]
        } else {
            effects
        }
    }
}

#[cfg(test)]
mod tests;
