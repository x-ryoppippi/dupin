use std::path::{Path, PathBuf};

use crate::item::Transfer;
use crate::{Action, CommandId, Page};

use super::{
    Destination, Effect, Intent, MAX_PAGE_DEPTH, Renaming, Results, SearchLocation, SearchState, Session,
    Visit,
};

impl Renaming {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Why Enter can't rename to the proposed name.
    pub fn problem(&self) -> Option<&'static str> {
        crate::file_name::problem(&self.name)
    }

    fn unchanged(&self) -> bool {
        self.path.file_name().is_some_and(|name| *name == *self.name)
    }
}

impl Destination {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn transfer(&self) -> Transfer {
        self.transfer
    }
}

/// The view an action was chosen on, so that what it asks for once it has run, such as opening
/// another view, lands only while that view is still the one on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Anchor {
    command: CommandId,
    /// The command whose row was selected, which searches a view the action opens.
    origin: CommandId,
    navigation: u64,
}

/// What became of an action's request to move between views or change one.
#[derive(Debug, PartialEq, Eq)]
pub enum Navigated {
    Done(Vec<Effect>),
    /// The user has moved on from the view the action was chosen on.
    Stale,
    /// Opening another view would stack more than the session keeps.
    TooDeep,
    /// The action wasn't chosen on a view it could close or change.
    NoPage,
}

impl Session {
    /// Where an action chosen now was chosen.
    pub fn anchor(&self) -> Anchor {
        Anchor {
            command: self.search.command.clone(),
            origin: self
                .search
                .origins
                .get(self.search.selected)
                .cloned()
                .unwrap_or_else(|| self.search.command.clone()),
            navigation: self.navigation,
        }
    }

    /// Whether `anchor`'s view is still the one on screen.
    pub fn is_current(&self, anchor: &Anchor) -> bool {
        anchor.navigation == self.navigation
            && anchor.command == self.search.command
            && self.renaming.is_none()
            && self.destination.is_none()
    }

    /// How many views are open above the command's own results.
    pub fn depth(&self) -> usize {
        self.visits
            .iter()
            .filter(|visit| matches!(visit.location, SearchLocation::Page(_)))
            .count()
            + usize::from(self.page().is_some())
    }

    /// Opens `page` above `anchor`'s view, searched by the command whose row the action was
    /// chosen from.
    pub fn open_page(&mut self, anchor: &Anchor, page: Page) -> Navigated {
        if !self.is_current(anchor) {
            return Navigated::Stale;
        }
        if self.depth() >= MAX_PAGE_DEPTH {
            return Navigated::TooDeep;
        }
        self.remember_query(true);
        Navigated::Done(self.visit(anchor.origin.clone(), SearchLocation::Page(page)))
    }

    /// Closes `anchor`'s view, as Esc does.
    pub fn close_page(&mut self, anchor: &Anchor) -> Navigated {
        if !self.is_current(anchor) {
            return Navigated::Stale;
        }
        if self.page().is_none() {
            return Navigated::NoPage;
        }
        Navigated::Done(self.leave_view())
    }

    /// Gives `anchor`'s view `state` and searches it again.
    pub fn replace_page_state(&mut self, anchor: &Anchor, state: serde_json::Value) -> Navigated {
        if !self.is_current(anchor) {
            return Navigated::Stale;
        }
        let SearchLocation::Page(page) = &mut self.search.location else {
            return Navigated::NoPage;
        };
        page.context = Some(state);
        self.navigation += 1;
        Navigated::Done(self.refresh())
    }

    /// Searches `anchor`'s view again.
    pub fn refresh_page(&mut self, anchor: &Anchor) -> Navigated {
        if !self.is_current(anchor) {
            return Navigated::Stale;
        }
        Navigated::Done(self.refresh())
    }

    pub(super) fn begin_destination(&mut self, path: PathBuf, transfer: Transfer) -> Vec<Effect> {
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

    pub(super) fn begin_rename(&mut self, path: PathBuf) -> Vec<Effect> {
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

    pub(super) fn rename_intent(&mut self, intent: Intent) -> Option<Vec<Effect>> {
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

    pub(super) fn destination_intent(&mut self, intent: Intent) -> Option<Vec<Effect>> {
        match intent {
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
        }
    }

    pub(super) fn browse(&mut self, folder: PathBuf) -> Vec<Effect> {
        let Some(browser) = self.commands.browser().cloned() else {
            return Vec::new();
        };
        self.visit(browser, SearchLocation::Folder(folder))
    }

    pub(super) fn push(&mut self, page: Page) -> Vec<Effect> {
        let previous_pages = self
            .visits
            .iter()
            .filter(|visit| matches!(visit.location, SearchLocation::Page(_)))
            .count();
        if previous_pages + usize::from(self.page().is_some()) >= MAX_PAGE_DEPTH {
            return Vec::new();
        }
        let Some(origin) = self.search.origins.get(self.search.selected).cloned() else {
            return Vec::new();
        };
        self.remember_query(true);
        self.visit(origin, SearchLocation::Page(page))
    }

    fn visit(&mut self, command: CommandId, location: SearchLocation) -> Vec<Effect> {
        let selected = self
            .selected_item()
            .map(|item| (self.search.origins[self.search.selected].clone(), item.id.clone()));
        self.navigation += 1;
        self.visits.push(Visit {
            location: std::mem::replace(&mut self.search.location, location),
            command: std::mem::replace(&mut self.search.command, command),
            explicit: std::mem::replace(&mut self.search.explicit, true),
            query: std::mem::take(&mut self.search.query),
            selected,
        });
        let mut effects = vec![Effect::ClearQuery];
        effects.extend(self.replace_search());
        effects
    }

    pub(super) fn leave_view(&mut self) -> Vec<Effect> {
        let visit = self.visits.pop().expect("returning to a previous view");
        self.navigation += 1;
        self.search.location = visit.location;
        self.search.command = visit.command;
        self.search.explicit = visit.explicit;
        self.search.query = visit.query;
        let mut effects = vec![Effect::ReplaceQuery(self.search.query.clone())];
        effects.extend(self.replace_search());
        self.search.selection_held = visit.selected.is_some();
        self.search.refresh_selection = visit.selected;
        effects
    }

    fn replace_search(&mut self) -> Vec<Effect> {
        self.search.recalled = None;
        self.search.deleting = false;
        self.search.before_deletion = None;
        let was_searching = matches!(self.search.results, Results::Searching { .. });
        self.clear_results();
        let effects = self.search();
        if was_searching && effects.is_empty() {
            vec![Effect::CancelSearch]
        } else {
            effects
        }
    }
}
