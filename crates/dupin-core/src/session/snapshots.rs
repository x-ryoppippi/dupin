use crate::CommandId;

use super::{Effect, Results, SearchLocation, Session};

impl Session {
    /// Capture only settled content belonging to the current search generation.
    pub fn capture(&self) -> Result<crate::snapshots::Snapshot, String> {
        if self.destination.is_some() {
            return Err("Finish choosing a destination before saving.".into());
        }
        if self.page().is_some() {
            return Err("A view opened from a result can’t be saved yet.".into());
        }
        if self.command().private_results {
            return Err("This search cannot be saved as a snapshot.".into());
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
        if self
            .commands
            .command(&command)
            .is_none_or(|spec| spec.private_results)
        {
            return Err("The source command is unavailable. Saved content remains readable.".into());
        }
        if self.commands.browser() == Some(&command) {
            return Err("A browsed folder can't be resumed. Saved content remains readable.".into());
        }
        self.clear_results();
        self.renaming = None;
        self.destination = None;
        self.visits.clear();
        self.search.location = SearchLocation::Command;
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
}
