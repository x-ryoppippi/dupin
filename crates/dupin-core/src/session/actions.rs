use crate::{Action, ActionDisposition, Item, ItemAction, Payload};

use super::{Effect, FollowupTarget, Intent, Session};

impl FollowupTarget {
    pub fn is_current(&self, session: &Session) -> bool {
        &self.command == session.command_id() && self.query == session.query()
    }
}

impl Session {
    pub(super) fn actions(&mut self, confirmation: Option<String>) -> Vec<Effect> {
        let effects = self
            .actionable_items()
            .get(self.search.selected)
            .filter(|item| item.has_actions())
            .map(|_| {
                vec![Effect::ShowActions {
                    id: self.result_key(self.search.selected),
                    generation: self.search.generation,
                    confirmation,
                }]
            })
            .unwrap_or_default();
        if !effects.is_empty() {
            self.search.selection_held = true;
            self.search.refresh_selection = None;
        }
        effects
    }

    pub(super) fn open(&mut self, intent: Intent) -> Vec<Effect> {
        let Some(item) = self.actionable_items().get(self.search.selected) else {
            return Vec::new();
        };
        let id = if intent == Intent::Open {
            item.primary.as_deref()
        } else {
            item.alternate.as_deref()
        };
        let Some(id) = id else {
            return if intent == Intent::Open {
                self.actions(None)
            } else {
                Vec::new()
            };
        };
        let Some(action) = item.item_action(id).cloned() else {
            return Vec::new();
        };
        if action.confirmation().is_some() {
            return self.actions(Some(action.id));
        }
        let item = item.clone();
        self.activate(&item, action)
    }

    fn activate(&mut self, item: &Item, action: ItemAction) -> Vec<Effect> {
        if self.destination.is_some()
            || action.confirmation().is_some()
            || matches!(action.action, Action::ChooseApplication(_))
        {
            return Vec::new();
        }
        match action.action {
            Action::Browse(folder) => return self.browse(folder),
            Action::Push(page) => return self.push(page),
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
        let dismiss = action.disposition == ActionDisposition::Dismiss;
        let picked = dismiss || matches!(action.action, Action::RunScript(_));
        let mut effects = vec![Effect::Run(action.action)];
        if picked {
            if item.record_usage {
                self.history
                    .write()
                    .unwrap()
                    .record(&item.id, &self.search.query, (self.clock)());
                effects.push(Effect::SaveHistory);
            }
            self.remember_query(true);
        }
        if dismiss {
            effects.extend(self.dismiss());
        }
        effects
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

    /// Open a link pressed in the detail of the row with result key `key`. Only a web link
    /// the detail itself shows opens.
    pub fn open_link(&mut self, key: &str, url: &str) -> Vec<Effect> {
        if crate::detail::web_link(url).is_none()
            || !self.select_key(key)
            || !self
                .detail(self.search.selected)
                .is_some_and(|detail| detail.body.links_to(url))
        {
            return Vec::new();
        }
        let mut effects = vec![Effect::Run(Action::OpenUrl(url.to_owned()))];
        self.remember_query(true);
        effects.extend(self.dismiss());
        effects
    }

    pub fn followup_target(&self, parent: &str) -> Option<FollowupTarget> {
        let item = self.actionable_items().get(self.selected())?;
        item.item_action(parent)?;
        Some(FollowupTarget {
            item: item.clone(),
            command: self.command_id().clone(),
            query: self.query().to_owned(),
        })
    }

    pub fn run_followup(&mut self, target: FollowupTarget, action: ItemAction) -> Vec<Effect> {
        if !target.is_current(self) {
            return Vec::new();
        }
        self.activate(&target.item, action)
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

    pub fn run_key_receiver(&mut self, key: &str, payload: &Payload, action: ItemAction) -> Vec<Effect> {
        if !self.select_key(key) {
            return Vec::new();
        }
        let item = self.search.items[self.search.selected].clone();
        if &item.payload != payload {
            return Vec::new();
        }
        self.activate(&item, action)
    }
}
