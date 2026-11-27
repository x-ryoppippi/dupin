use std::sync::atomic::Ordering;

use dupin_core::{Intent, ItemAction, Session};
use rune::{Key, KeyEvent, Tx};

use super::{ICON_SIZE, Ui};

pub const SEND_TO: &str = "Send to…";

/// Leave out the actions whose condition doesn't hold now: an app's that need it running, by
/// `running`. They are checked as the action screen opens, never as rows are built.
fn offered(actions: &mut Vec<ItemAction>, running: impl Fn(&std::path::Path) -> bool) {
    actions.retain(|action| match action.condition() {
        None => true,
        Some(dupin_core::Condition::Running(path)) => running(path),
    });
}

pub struct ActionScreen {
    pub key: String,
    pub item: dupin_core::Item,
    pub selected: usize,
    pub stage: ActionStage,
}

#[derive(Clone, Debug)]
pub enum ApplicationState {
    Loading,
    Ready(Vec<dupin_mac::workspace::Application>),
    Failed(String),
}

#[derive(Clone, Debug)]
pub enum ActionStage {
    List,
    Applications {
        parent: String,
        token: u64,
        state: ApplicationState,
        selected: usize,
    },
    Receivers {
        actions: Vec<ItemAction>,
        selected: usize,
    },
    Confirm {
        action_id: String,
        selected: usize,
        from_list: bool,
    },
}

#[derive(Debug, PartialEq, Eq)]
enum ActionKey {
    Stay,
    Back,
    Run(String, String),
    Confirm,
    Choice(usize),
    SendTo,
    Receive(usize),
}

impl ActionScreen {
    /// The screen for the row `item`, whose result key is `key`, listing what its kind doesn't
    /// hide; asking to confirm `confirmation` opens on that action's prompt, as Enter does on
    /// an action that needs one, hidden or not.
    fn new(key: String, mut item: dupin_core::Item, confirmation: Option<String>) -> Option<Self> {
        item.actions
            .retain(|action| action.is_listed() || confirmation.as_ref() == Some(&action.id));
        let (selected, stage) = if let Some(action_id) = confirmation {
            let selected = item.actions.iter().position(|action| action.id == action_id)?;
            crate::trace::action_screen("confirm");
            (
                selected,
                ActionStage::Confirm {
                    action_id,
                    selected: 0,
                    from_list: false,
                },
            )
        } else {
            (0, ActionStage::List)
        };
        Some(Self {
            key,
            item,
            selected,
            stage,
        })
    }

    fn accept_applications(
        &mut self,
        key: &str,
        token: u64,
        result: Result<Vec<dupin_mac::workspace::Application>, String>,
    ) -> bool {
        if self.key != key {
            return false;
        }
        let ActionStage::Applications {
            token: current,
            state,
            ..
        } = &mut self.stage
        else {
            return false;
        };
        if *current != token {
            return false;
        }
        *state = match result {
            Ok(apps) => ApplicationState::Ready(apps),
            Err(error) => ApplicationState::Failed(error),
        };
        true
    }

    fn selected_index(&self) -> usize {
        match &self.stage {
            ActionStage::Applications { selected, .. }
            | ActionStage::Receivers { selected, .. }
            | ActionStage::Confirm { selected, .. } => *selected,
            ActionStage::List => self.selected,
        }
    }

    fn action_count(&self) -> usize {
        self.item.actions.len() + usize::from(!self.item.payload.is_empty())
    }

    fn back(&mut self) -> ActionKey {
        if matches!(
            self.stage,
            ActionStage::List | ActionStage::Confirm { from_list: false, .. }
        ) {
            return ActionKey::Back;
        }
        self.stage = ActionStage::List;
        crate::trace::action_screen("list");
        ActionKey::Stay
    }

    fn key(&mut self, key: &Key, repeat: bool) -> ActionKey {
        if matches!(key, Key::Enter) && repeat {
            return ActionKey::Stay;
        }
        if matches!(key, Key::Left | Key::Escape) {
            return self.back();
        }
        match &mut self.stage {
            ActionStage::Confirm { selected, .. } => {
                match key {
                    Key::Up => *selected = 0,
                    Key::Down => *selected = 1,
                    Key::Enter if *selected == 1 => return ActionKey::Confirm,
                    Key::Enter => return self.back(),
                    _ => {}
                }
                return ActionKey::Stay;
            }
            ActionStage::Applications { state, selected, .. } => {
                if let ApplicationState::Ready(apps) = state {
                    match key {
                        Key::Up => *selected = selected.saturating_sub(1),
                        Key::Down => *selected = (*selected + 1).min(apps.len().saturating_sub(1)),
                        Key::Enter if !apps.is_empty() => return ActionKey::Choice(*selected),
                        _ => {}
                    }
                }
                return ActionKey::Stay;
            }
            ActionStage::Receivers { actions, selected } => {
                match key {
                    Key::Up => *selected = selected.saturating_sub(1),
                    Key::Down => *selected = (*selected + 1).min(actions.len().saturating_sub(1)),
                    Key::Enter if !actions.is_empty() => return ActionKey::Receive(*selected),
                    _ => {}
                }
                return ActionKey::Stay;
            }
            ActionStage::List => {}
        }
        match key {
            Key::Up => self.selected = self.selected.saturating_sub(1),
            Key::Down => self.selected = (self.selected + 1).min(self.action_count().saturating_sub(1)),
            Key::Left | Key::Escape => return ActionKey::Back,
            Key::Enter => {
                if let Some(action) = self.item.actions.get(self.selected) {
                    return ActionKey::Run(self.key.clone(), action.id.clone());
                }
                if self.selected < self.action_count() {
                    return ActionKey::SendTo;
                }
            }
            _ => {}
        }
        ActionKey::Stay
    }

    pub(super) fn is_current(&self, session: &Session) -> bool {
        session
            .items()
            .iter()
            .enumerate()
            .any(|(index, _)| session.result_key(index) == self.key)
    }
}

impl Ui {
    pub(super) fn show_action_screen(&mut self, id: String, confirmation: Option<String>) {
        let Some(item) = self
            .session
            .items()
            .iter()
            .enumerate()
            .find_map(|(i, item)| (self.session.result_key(i) == id).then_some(item))
        else {
            return;
        };
        let mut item = item.clone();
        if item.actions.iter().any(|action| action.condition().is_some()) {
            let running = crate::features::apps::running();
            offered(&mut item.actions, |path| {
                dupin_mac::running_apps::is_running(&running, path)
            });
        }
        if let Some(screen) = ActionScreen::new(id, item, confirmation) {
            self.action_screen = Some(screen);
        }
    }

    pub fn actions(&mut self, id: &str, tx: &mut Tx) {
        if self.picker.is_some() {
            return;
        }
        if self.session.select_key(id) {
            let effects = self.session.intent(Intent::Actions).unwrap_or_default();
            self.apply(effects, tx);
        }
    }

    pub fn action_back(&mut self, tx: &mut Tx) {
        if let Some(screen) = &mut self.action_screen
            && screen.back() == ActionKey::Back
        {
            self.close_action_screen(tx);
        }
    }

    fn close_action_screen(&mut self, tx: &mut Tx) {
        self.action_screen = None;
        tx.focus("query", false);
    }

    pub fn pick_action(&mut self, key: &str, action: &str, tx: &mut Tx) {
        let Some(screen) = self
            .action_screen
            .as_ref()
            .filter(|screen| screen.key == key && screen.is_current(&self.session))
        else {
            return;
        };
        let Some(item_action) = screen.item.item_action(action) else {
            return;
        };
        if !matches!(screen.stage, ActionStage::List) {
            return;
        }
        let selected = screen
            .item
            .actions
            .iter()
            .position(|candidate| candidate.id == action)
            .unwrap();
        if let Some(dupin_core::Action::ChooseApplication(path)) = screen.item.action(action) {
            let path = path.clone();
            self.next_lookup += 1;
            let token = self.next_lookup;
            self.action_screen.as_mut().unwrap().stage = ActionStage::Applications {
                parent: action.into(),
                token,
                state: ApplicationState::Loading,
                selected: 0,
            };
            self.action_screen.as_mut().unwrap().selected = selected;
            crate::trace::action_screen("applications loading");
            let me = self.me.clone();
            let key = key.to_owned();
            let running = self.running.clone();
            running.fetch_add(1, Ordering::AcqRel);
            std::thread::spawn(move || {
                let result = dupin_mac::workspace::applications_for(&path);
                me.update(move |ui, _| ui.applications_found(&key, token, result));
                running.fetch_sub(1, Ordering::AcqRel);
            });
        } else if item_action.confirmation().is_some() {
            self.action_screen.as_mut().unwrap().stage = ActionStage::Confirm {
                action_id: action.into(),
                selected: 0,
                from_list: true,
            };
            self.action_screen.as_mut().unwrap().selected = selected;
            crate::trace::action_screen("confirm");
        } else {
            self.action_screen = None;
            let effects = self.session.run_key_action(key, action);
            self.apply(effects, tx);
        }
    }

    fn applications_found(
        &mut self,
        key: &str,
        token: u64,
        result: Result<Vec<dupin_mac::workspace::Application>, String>,
    ) {
        let Some(screen) = self
            .action_screen
            .as_mut()
            .filter(|screen| screen.key == key && screen.is_current(&self.session))
        else {
            return;
        };
        if !screen.accept_applications(key, token, result) {
            return;
        }
        if let (
            ActionStage::Applications {
                state: ApplicationState::Ready(apps),
                ..
            },
            Some(cache),
        ) = (&screen.stage, &self.icon_cache)
        {
            for app in apps {
                self.icons
                    .entry(dupin_core::Icon::File(app.path.clone()))
                    .or_insert_with(|| cache.app_icon(&app.path, ICON_SIZE));
            }
        }
        crate::trace::action_screen(
            if matches!(
                screen.stage,
                ActionStage::Applications {
                    state: ApplicationState::Ready(_),
                    ..
                }
            ) {
                "applications ready"
            } else {
                "applications failed"
            },
        );
    }

    fn confirm_action(&mut self, tx: &mut Tx) {
        let Some(screen) = &self.action_screen else { return };
        let ActionStage::Confirm {
            action_id,
            selected: 1,
            ..
        } = &screen.stage
        else {
            return;
        };
        let effects = self.session.run_confirmed_key_action(&screen.key, action_id);
        self.action_screen = None;
        self.apply(effects, tx);
    }

    pub fn select_confirmation(&mut self, selected: usize, tx: &mut Tx) {
        if let Some(screen) = &mut self.action_screen
            && let ActionStage::Confirm { selected: index, .. } = &mut screen.stage
        {
            *index = selected.min(1);
            if selected == 0 {
                self.action_back(tx);
            } else {
                self.confirm_action(tx);
            }
        }
    }

    pub fn pick_application(&mut self, index: usize, tx: &mut Tx) {
        let Some(screen) = &self.action_screen else { return };
        let ActionStage::Applications {
            parent,
            state: ApplicationState::Ready(apps),
            ..
        } = &screen.stage
        else {
            return;
        };
        let Some(app) = apps.get(index) else { return };
        let Some(dupin_core::Action::ChooseApplication(path)) = screen.item.action(parent) else {
            return;
        };
        let effects = self.session.run_key_choice(
            &screen.key,
            parent,
            dupin_core::Action::OpenWith {
                path: path.clone(),
                application: app.path.clone(),
            },
        );
        self.action_screen = None;
        self.apply(effects, tx);
    }

    pub fn show_receivers(&mut self, tx: &mut Tx) {
        let Some(screen) = self
            .action_screen
            .as_mut()
            .filter(|screen| matches!(screen.stage, ActionStage::List) && screen.is_current(&self.session))
        else {
            return;
        };
        let web = self.web.read().unwrap().clone();
        let held = web.held();
        let actions = crate::receivers::for_payload(
            &self.kinds,
            &screen.item.payload,
            held.as_deref().unwrap_or_default(),
        );
        if actions.is_empty() {
            return;
        }
        screen.selected = screen.item.actions.len();
        screen.stage = ActionStage::Receivers { actions, selected: 0 };
        crate::trace::action_screen("receivers");
        tx.scroll_into_view("action-0");
        if held.is_some() || screen.item.payload.nonblank_text().is_none() {
            return;
        }
        // The catalogues aren't loaded, are being filled, or were loaded for other settings, so
        // their web searches join the list once a package has answered, as typing waits for them.
        let key = screen.key.clone();
        let me = self.me.clone();
        let running = self.running.clone();
        running.fetch_add(1, Ordering::AcqRel);
        std::thread::spawn(move || {
            let searches = web.searches(crate::extensions::deadline_in(crate::extensions::QUERY_DEADLINE));
            me.update(move |ui, _| ui.web_receivers_found(&key, &web, &searches));
            running.fetch_sub(1, Ordering::AcqRel);
        });
    }

    fn web_receivers_found(
        &mut self,
        key: &str,
        web: &crate::web::WebSearches,
        searches: &[(dupin_core::Item, dupin_js_runtime::web_search::WebSearch)],
    ) {
        if *self.web.read().unwrap() != *web {
            return;
        }
        let Some(screen) = self
            .action_screen
            .as_mut()
            .filter(|screen| screen.key == key && screen.is_current(&self.session))
        else {
            return;
        };
        if let ActionStage::Receivers { actions, .. } = &mut screen.stage {
            *actions = crate::receivers::for_payload(&self.kinds, &screen.item.payload, searches);
            crate::trace::action_screen("receivers with web searches");
        }
    }

    pub fn pick_receiver(&mut self, index: usize, tx: &mut Tx) {
        let Some(screen) = &self.action_screen else { return };
        let ActionStage::Receivers { actions, .. } = &screen.stage else {
            return;
        };
        let Some(action) = actions.get(index) else { return };
        let effects = self
            .session
            .run_key_receiver(&screen.key, &screen.item.payload, action.clone());
        self.apply(effects, tx);
    }

    pub(super) fn action_screen_key(&mut self, ev: &KeyEvent, tx: &mut Tx) -> bool {
        if let Some(screen) = &mut self.action_screen {
            match screen.key(&ev.key, tx.key_repeat()) {
                ActionKey::Back => self.close_action_screen(tx),
                ActionKey::Run(key, action) => self.pick_action(&key, &action, tx),
                ActionKey::Confirm => self.confirm_action(tx),
                ActionKey::Choice(index) => self.pick_application(index, tx),
                ActionKey::SendTo => self.show_receivers(tx),
                ActionKey::Receive(index) => self.pick_receiver(index, tx),
                ActionKey::Stay => {
                    tx.scroll_into_view(format!("action-{}", screen.selected_index()));
                }
            }
            return true;
        }
        false
    }
}

#[cfg(test)]
mod action_screen_tests {
    use super::*;
    use dupin_core::{
        Action, CommandId, CommandRegistry, CommandSpec, Delivery, Effect, History, Item, QueryPresentation,
    };
    use std::path::PathBuf;

    #[test]
    fn enter_never_bypasses_cancel_and_repeats_never_confirm() {
        let (_, mut screen) = fixture();
        screen.item.actions.push(
            dupin_core::ItemAction::new("trash", "Trash", Action::Trash("/fixture".into()))
                .confirm("Trash?", "Trash")
                .consumes(),
        );
        screen.selected = screen.item.actions.len() - 1;
        for _ in 0..50 {
            let action = screen.key(&Key::Enter, false);
            assert_ne!(action, ActionKey::Confirm);
            if matches!(action, ActionKey::Run(_, _)) {
                screen.stage = ActionStage::Confirm {
                    action_id: "trash".into(),
                    selected: 0,
                    from_list: true,
                };
            }
        }
        screen.stage = ActionStage::Confirm {
            action_id: "trash".into(),
            selected: 0,
            from_list: true,
        };
        screen.key(&Key::Down, false);
        assert_eq!(screen.key(&Key::Enter, true), ActionKey::Stay);
        assert_eq!(screen.key(&Key::Enter, false), ActionKey::Confirm);
    }

    #[test]
    fn substage_back_and_late_application_replies_keep_the_original_action() {
        let (_, mut screen) = fixture();
        screen.selected = 1;
        for stage in [
            ActionStage::Confirm {
                action_id: "trash".into(),
                selected: 0,
                from_list: true,
            },
            ActionStage::Applications {
                parent: "choose".into(),
                token: 8,
                state: ApplicationState::Loading,
                selected: 0,
            },
        ] {
            screen.stage = stage;
            assert_eq!(screen.key(&Key::Left, false), ActionKey::Stay);
            assert!(matches!(screen.stage, ActionStage::List));
            assert_eq!(screen.selected, 1);
        }
        screen.stage = ActionStage::Applications {
            parent: "choose".into(),
            token: 8,
            state: ApplicationState::Loading,
            selected: 0,
        };
        assert!(!screen.accept_applications("old-key", 8, Ok(vec![])));
        let key = screen.key.clone();
        assert!(!screen.accept_applications(&key, 7, Err("old failure".into())));
        assert!(screen.accept_applications(&key, 8, Ok(vec![])));
        assert_eq!(screen.key(&Key::Enter, false), ActionKey::Stay);
        assert!(screen.accept_applications(&key, 8, Err("Lookup failed".into())));
        assert!(
            matches!(&screen.stage, ActionStage::Applications { state: ApplicationState::Failed(error), .. } if error == "Lookup failed")
        );
    }

    fn fixture() -> (Session, ActionScreen) {
        fixture_with(Item::path(
            PathBuf::from("/fixture/example"),
            "Example".into(),
            vec![],
        ))
    }

    fn fixture_with(item: Item) -> (Session, ActionScreen) {
        let origin = CommandId::new("sample.search").unwrap();
        let mut registry = CommandRegistry::new(origin.clone(), None);
        registry
            .register(CommandSpec::new(
                origin.clone(),
                10,
                QueryPresentation {
                    placeholder: "Search".into(),
                    chip: None,
                    empty_hint: None,
                    no_results_hint: None,
                    show_searching: false,
                    backdrop: 0.0,
                },
            ))
            .unwrap();
        let mut session = Session::new(History::default(), registry).unwrap();
        let effects = session.set_query("example");
        let generation = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Search { generation, .. } => Some(*generation),
                _ => None,
            })
            .unwrap();
        session.deliver_from(
            origin,
            Delivery::Batch {
                generation,
                items: vec![item],
            },
        );
        session.intent(Intent::Actions);
        let screen = ActionScreen {
            key: session.result_key(0),
            item: session.items()[0].clone(),
            selected: 0,
            stage: ActionStage::List,
        };
        (session, screen)
    }

    #[test]
    fn an_action_that_needs_its_app_running_is_offered_only_while_it_runs() {
        use dupin_core::{Action, Condition, ItemAction};
        let safari = PathBuf::from("/Applications/Safari.app");
        let actions = || {
            vec![
                ItemAction::new("open", "Open", Action::Open(safari.clone())),
                ItemAction::new("hide", "Hide", Action::Open(safari.clone()))
                    .when(Condition::Running(safari.clone())),
            ]
        };
        let ids = |actions: Vec<ItemAction>| actions.into_iter().map(|action| action.id).collect::<Vec<_>>();
        let mut running = actions();
        offered(&mut running, |path| path == safari);
        assert_eq!(ids(running), ["open", "hide"]);
        let mut quit = actions();
        offered(&mut quit, |_| false);
        assert_eq!(ids(quit), ["open"]);
    }

    #[test]
    fn keyboard_navigation_is_bounded_and_runs_the_captured_key() {
        let (_, mut screen) = fixture();
        screen.key(&Key::Up, false);
        assert_eq!(screen.selected, 0);
        for _ in 0..10 {
            screen.key(&Key::Down, false);
        }
        assert_eq!(screen.selected, 2);
        screen.key(&Key::Up, false);
        assert_eq!(
            screen.key(&Key::Enter, false),
            ActionKey::Run(screen.key.clone(), "reveal".into())
        );
        assert_eq!(screen.key(&Key::Text("x".into()), false), ActionKey::Stay);
    }

    #[test]
    fn cancel_returns_without_changing_search_or_selection() {
        let (session, mut screen) = fixture();
        let before = (
            session.query().to_owned(),
            session.selected(),
            session.result_key(0),
        );
        for key in [Key::Left, Key::Escape, Key::Enter] {
            screen.stage = ActionStage::Confirm {
                action_id: "restart".into(),
                selected: 0,
                from_list: false,
            };
            assert_eq!(screen.key(&key, false), ActionKey::Back);
            assert_eq!(
                (
                    session.query().to_owned(),
                    session.selected(),
                    session.result_key(0)
                ),
                before
            );
        }
    }

    #[test]
    fn generation_changes_and_reset_make_captured_actions_inert() {
        for reset in [false, true] {
            let (mut session, screen) = fixture();
            assert!(screen.is_current(&session));
            if reset {
                session.reset();
            } else {
                session.set_query("another");
            }
            assert!(!screen.is_current(&session));
            assert!(session.run_key_action(&screen.key, "open").is_empty());
        }
    }

    #[test]
    fn enter_uses_the_selected_action_without_running_external_effects() {
        let (mut session, mut screen) = fixture();
        screen.selected = 1;
        let ActionKey::Run(key, action) = screen.key(&Key::Enter, false) else {
            panic!("expected action")
        };
        let effects = session.run_key_action(&key, &action);
        assert!(
            effects.iter().any(
                |effect| matches!(effect, Effect::Run(Action::Reveal(path)) if path == std::path::Path::new("/fixture/example"))
            )
        );
    }

    #[test]
    fn send_to_navigation_preserves_the_parent_selection() {
        let (_, mut screen) = fixture();
        screen.selected = screen.item.actions.len();
        assert_eq!(screen.key(&Key::Enter, false), ActionKey::SendTo);
        screen.stage = ActionStage::Receivers {
            actions: crate::receivers::for_payload(&dupin_mac::kinds::native(), &screen.item.payload, &[]),
            selected: 0,
        };
        assert_eq!(screen.key(&Key::Enter, true), ActionKey::Stay);
        screen.key(&Key::Down, false);
        assert_eq!(screen.key(&Key::Enter, false), ActionKey::Receive(1));
        assert_eq!(screen.key(&Key::Escape, false), ActionKey::Stay);
        assert!(matches!(screen.stage, ActionStage::List));
        assert_eq!(screen.key(&Key::Enter, false), ActionKey::SendTo);
    }

    #[test]
    fn a_hidden_action_leaves_the_list_but_keeps_its_route_and_shortcut() {
        use dupin_core::kind::QUICK_LOOK;

        let kinds = dupin_mac::kinds::native();
        kinds.configure(
            &dupin_core::Config::parse(
                "[advanced.kinds.file]\nhide = [\"open\", \"quick-look\"]\norder = [\"reveal\"]",
            )
            .unwrap()
            .advanced
            .kinds,
        );
        let path = PathBuf::from("/fixture/example.txt");
        let item = kinds.file(path.clone(), "example.txt".into(), vec![], false);
        let (mut session, _) = fixture_with(item.clone());
        let key = session.result_key(0);

        let screen = ActionScreen::new(key.clone(), item.clone(), None).unwrap();
        let listed: Vec<_> = screen
            .item
            .actions
            .iter()
            .map(|action| action.id.as_str())
            .collect();
        assert_eq!(
            listed,
            [
                "reveal",
                "open-with",
                "copy-name",
                "rename",
                "move-to",
                "copy-to",
                "duplicate",
                "compress",
                "trash"
            ]
        );
        assert_eq!(
            ActionScreen::new(key.clone(), item, Some("open".into()))
                .unwrap()
                .item
                .item_action("open")
                .map(|action| action.title.as_str()),
            Some("Open"),
            "a hidden action its route confirms is shown for the prompt"
        );

        assert_eq!(
            session.intent(Intent::Open).unwrap().first(),
            Some(&Effect::Run(Action::Open(path.clone()))),
            "Enter still opens"
        );
        let (mut session, _) = fixture_with(kinds.file(path.clone(), "example.txt".into(), vec![], false));
        assert_eq!(
            session.intent(Intent::OpenAlternate).unwrap().first(),
            Some(&Effect::Run(Action::Reveal(path.clone())))
        );
        let (mut session, _) = fixture_with(kinds.file(path.clone(), "example.txt".into(), vec![], false));
        let key = session.result_key(0);
        assert_eq!(
            session.run_key_action(&key, QUICK_LOOK).first(),
            Some(&Effect::Run(Action::Preview(path))),
            "⌘Y still previews"
        );
    }
}
