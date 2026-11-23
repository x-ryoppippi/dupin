//! The panel's state: a `Session` from dupin-core plus what only the UI needs (the text
//! field, icons, a notice). Handlers translate keys into session intents and carry out the
//! effects the session returns.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use dupin_core::{
    Backdrop, CommandId, Config, Delivery, Effect, History, InputSource, Intent, Position, Searcher, Session,
};
use dupin_mac::{AppsProvider, AppsWatcher, quick_look::Previewer};
use rune::{
    Align, AppHandle, DisplayTarget, Image, ImageCache, Key, KeyEvent, OverlayPlacement, ScreenEdge,
    TextEdit, Tx, Waker,
};

/// Icons kept decoded: enough for a full file result list and the apps around it.
const ICON_CAP: usize = 512;
/// The panel's top edge below the display's for `position = "top"`: about where Spotlight
/// puts its field.
const TOP_MARGIN: f32 = 180.0;
pub const ICON_SIZE: f32 = 32.0;

fn mark_query_qualifiers(edit: &mut TextEdit, command: &CommandId) {
    let marks: Vec<_> = crate::commands::query_qualifier_ranges(command, &edit.text)
        .into_iter()
        .filter(|range| {
            ![edit.anchor, edit.focus]
                .into_iter()
                .any(|position| range.start < position && position < range.end)
        })
        .map(|range| rune::MarkRange::new(range, rune::Mark::Pill("file-qualifier".into())))
        .collect();
    if edit.marks() != marks.as_slice() {
        edit.set_marks(marks);
    }
}

pub struct Windows {
    pub panel: AppHandle<Ui>,
    pub prefs: AppHandle<crate::prefs::Prefs>,
    pub snapshots: AppHandle<crate::snapshots::Snapshots>,
}

pub struct Configuration {
    pub values: Rc<RefCell<Config>>,
    pub hotkey: Arc<crate::hotkey::Binding>,
    pub files: Arc<crate::settings::Service>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Restart {
    Update,
    Requested,
}

pub struct ActionScreen {
    pub key: String,
    pub item: dupin_core::Item,
    pub layout_key: String,
    pub selected: usize,
}

#[derive(Debug, PartialEq, Eq)]
enum ActionKey {
    Stay,
    Back,
    Run(String, String),
}

impl ActionScreen {
    fn key(&mut self, key: &Key) -> ActionKey {
        match key {
            Key::Up => self.selected = self.selected.saturating_sub(1),
            Key::Down => self.selected = (self.selected + 1).min(self.item.actions.len().saturating_sub(1)),
            Key::Left | Key::Escape => return ActionKey::Back,
            Key::Enter => {
                if let Some(action) = self.item.actions.get(self.selected) {
                    return ActionKey::Run(self.key.clone(), action.id.clone());
                }
            }
            _ => {}
        }
        ActionKey::Stay
    }

    fn is_current(&self, session: &Session) -> bool {
        session
            .items()
            .iter()
            .enumerate()
            .any(|(index, _)| session.result_key(index) == self.key)
    }
}

pub struct Ui {
    pub action_screen: Option<ActionScreen>,
    pub session: Session,
    pub query: TextEdit,
    /// A problem to show above the results until the panel next hides (a hotkey that
    /// couldn't be registered, a broken config file).
    pub notice: Option<String>,
    snapshot_notice_until: Option<f64>,
    pub config_problem: Option<String>,
    pub registration_notice: Option<String>,
    configuration: Configuration,
    quick_look: Rc<RefCell<Option<Previewer>>>,
    acknowledged_config: Config,
    pub icons: HashMap<String, Image>,
    /// Between `show` and `hide`: the backdrop animates only then.
    pub shown: bool,
    /// The clock when the panel last showed, and when the query last changed.
    pub shown_at: f64,
    pub typed_at: f64,
    pub backdrop: crate::shader::Source,
    backdrop_pick: Backdrop,
    icon_cache: Option<ImageCache>,
    searchers: HashMap<CommandId, Searcher>,
    web: Arc<RwLock<crate::web::WebCatalog>>,
    file_demotions: Arc<RwLock<Vec<String>>>,
    deadlines: HashMap<CommandId, (u64, f64)>,
    _apps_watcher: Option<AppsWatcher>,
    saver: crate::store::Saver,
    position: Position,
    input_source: InputSource,
    /// The input source to put back when the panel hides, if showing it switched one.
    restore_input: Option<dupin_mac::input_source::Previous>,
    /// An update was staged while the panel showed: restart into it when the panel hides.
    relaunch: Option<Restart>,
    settings_after_close: bool,
    snapshots_after_close: bool,
    /// Actions handed to Launch Services that haven't returned yet; a relaunch waits for them.
    running: Arc<AtomicUsize>,
    updater: crate::update::Updater,
    me: AppHandle<Ui>,
    prefs: AppHandle<crate::prefs::Prefs>,
    snapshots: AppHandle<crate::snapshots::Snapshots>,
}

impl Ui {
    /// Cursor-only redraws do not emit Rune input-change callbacks.
    pub fn query_display(&self) -> TextEdit {
        let mut query = self.query.clone();
        mark_query_qualifiers(&mut query, self.session.command_id());
        query
    }

    pub fn query_edit(&mut self) -> &mut TextEdit {
        mark_query_qualifiers(&mut self.query, self.session.command_id());
        &mut self.query
    }

    pub fn new(
        windows: Windows,
        updater: crate::update::Updater,
        configuration: Configuration,
        history: History,
        notice: Option<String>,
        quick_look: Rc<RefCell<Option<Previewer>>>,
    ) -> Ui {
        let Windows {
            panel: me,
            prefs,
            snapshots,
        } = windows;
        let config = configuration.values.borrow().clone();
        let apps_provider = Arc::new(AppsProvider::standard());
        let registration = crate::commands::register(&config, apps_provider.clone())
            .expect("built-in command registration is valid");
        let apps_watcher = apps_provider.watch({
            let me = me.clone();
            move || me.update(|s, tx| s.refresh_apps(tx))
        });
        let apps_watcher = match apps_watcher {
            Ok(watcher) => Some(watcher),
            Err(error) => {
                crate::log::warn(&format!("couldn't watch installed apps: {error}"));
                None
            }
        };
        // Start watching before the initial scan, so an install during startup can't be missed.
        let warm = apps_provider.clone();
        std::thread::spawn(move || warm.warm());
        let deliver = |me: AppHandle<Ui>, origin: CommandId| {
            move |d: Delivery| {
                let origin = origin.clone();
                me.update(move |s, tx| {
                    s.poll(tx.clock(), tx);
                    s.deliver(origin, d);
                });
            }
        };
        let backdrop = backdrop_source(&config.backdrop);
        let searchers = registration
            .providers
            .into_iter()
            .map(|(id, provider)| {
                let name = id.as_str().replace('.', "-");
                (
                    id.clone(),
                    Searcher::spawn(&name, provider, deliver(me.clone(), id)),
                )
            })
            .collect();
        let sources = registration.registry.ids();
        snapshots.update(move |s, _| s.set_sources(sources));
        let mut query = TextEdit::new("");
        query.focused = true;
        Ui {
            action_screen: None,
            acknowledged_config: config.clone(),
            session: Session::new(history, registration.registry)
                .expect("built-in command registration is valid"),
            saver: crate::store::Saver::spawn(),
            query,
            notice: None,
            snapshot_notice_until: None,
            config_problem: notice,
            registration_notice: registration.notice,
            icons: HashMap::new(),
            shown: false,
            shown_at: 0.0,
            typed_at: f64::NEG_INFINITY,
            backdrop,
            backdrop_pick: config.backdrop.clone(),
            icon_cache: None,
            searchers,
            web: registration.web,
            file_demotions: registration.file_demotions,
            deadlines: HashMap::new(),
            _apps_watcher: apps_watcher,
            position: config.position,
            input_source: config.input_source.clone(),
            configuration,
            restore_input: None,
            relaunch: None,
            settings_after_close: false,
            snapshots_after_close: false,
            running: Arc::new(AtomicUsize::new(0)),
            updater,
            quick_look,
            me,
            prefs,
            snapshots,
        }
    }

    pub fn apply_config(&mut self, config: &Config, tx: &mut Tx) {
        let previous_limit = self.session.command().result_limit;
        let ranking_changed = {
            let mut rules = self.file_demotions.write().unwrap();
            let changed = *rules != config.file_demotions;
            if changed {
                rules.clone_from(&config.file_demotions);
            }
            changed
        };
        let registration =
            crate::commands::configured(config).expect("built-in command registration is valid");
        let web_changed = {
            let mut web = self.web.write().unwrap();
            let changed = *web != registration.web;
            *web = registration.web;
            changed
        };
        let sources = registration.registry.ids();
        self.snapshots.update(move |s, _| s.set_sources(sources));
        self.session
            .set_commands(registration.registry)
            .expect("built-in command registration is valid");
        self.registration_notice = registration.notice;
        if self.session.command().result_limit != previous_limit
            || (web_changed && self.session.command_id().as_str() == crate::commands::APPS)
            || (ranking_changed && self.session.command_id().as_str() == crate::commands::FILES)
        {
            let effects = self.session.refresh();
            self.apply(effects, tx);
        }
        self.position = config.position;
        self.input_source = config.input_source.clone();
        self.updater.set_enabled(config.auto_update);
        if self.backdrop_pick != config.backdrop {
            self.backdrop = backdrop_source(&config.backdrop);
            self.backdrop_pick = config.backdrop.clone();
        }
    }

    pub fn reload_config(&mut self, update: crate::settings::Update, tx: &mut Tx) {
        if update.revision != self.configuration.files.revision() {
            return;
        }
        let previous = self.configuration.values.borrow().clone();
        let (snapshot, read_problem) = reconcile_snapshot(
            update.result,
            &self.acknowledged_config,
            update.save_problem.is_some(),
        );
        let accepted = snapshot.and_then(|config| {
            accept_config(config, &previous, |shortcut| {
                if self.configuration.hotkey.is_installed(shortcut) {
                    Ok(())
                } else {
                    self.configuration.hotkey.replace(shortcut)
                }
            })
        });
        let problem = match accepted {
            Ok(config) => {
                self.acknowledged_config = config.clone();
                if *self.configuration.values.borrow() != config {
                    *self.configuration.values.borrow_mut() = config.clone();
                    self.apply_config(&config, tx);
                }
                None
            }
            Err(error) => Some(error),
        };
        let problems: Vec<_> = [problem, read_problem, update.save_problem, update.watch_problem]
            .into_iter()
            .flatten()
            .collect();
        let problem = (!problems.is_empty()).then(|| problems.join("; "));
        if self.config_problem != problem
            && let Some(problem) = &problem
        {
            crate::log::warn(problem);
        }
        self.config_problem = problem.clone();
        let config = self.configuration.values.borrow().clone();
        self.prefs.update(move |prefs, _| {
            prefs.reflect_config(update.revision, config, update.read_only, problem);
        });
    }

    /// An update is staged: restart into it now, or once neither the panel nor Settings shows.
    pub fn updated(&mut self, tx: &mut Tx) {
        self.restart(Restart::Update, tx);
    }

    /// Settings' explicit relaunch, installing a staged update first when present.
    pub fn relaunch(&mut self, tx: &mut Tx) {
        self.restart(Restart::Requested, tx);
    }

    pub fn resume_restart(&mut self, tx: &mut Tx) {
        if let Some(request) = self.relaunch.take() {
            self.restart(request, tx);
        }
    }

    /// The decision is made here, on the UI thread, so it sees the windows as they are now.
    /// Automatic updates wait for both windows to hide; an explicit request closes them.
    fn restart(&mut self, request: Restart, tx: &mut Tx) {
        if request == Restart::Update
            && (self.me.is_visible() || self.prefs.is_visible() || self.snapshots.is_visible())
        {
            self.relaunch = Some(request);
        } else if self.running.load(Ordering::Acquire) > 0 {
            self.relaunch_when_idle(request);
        } else {
            match self.updater.install_staged() {
                Ok(installed) => {
                    if installed || request == Restart::Requested {
                        tx.relaunch();
                    }
                }
                Err(error) => {
                    crate::log::warn(&error);
                    if request == Restart::Requested {
                        self.show_notice(error, tx);
                    }
                }
            }
        }
    }

    /// Ask again once every action already handed off has been carried out, so exiting
    /// doesn't cut short the app or file the user just opened. The panel may have been
    /// summoned meanwhile, so automatic updates check visibility again.
    fn relaunch_when_idle(&self, request: Restart) {
        let running = self.running.clone();
        let me = self.me.clone();
        std::thread::spawn(move || {
            while running.load(Ordering::Acquire) > 0 {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            me.update(move |s, tx| s.restart(request, tx));
        });
    }

    pub fn setup(s: &mut Ui, waker: Waker) {
        s.icon_cache = Some(ImageCache::new(waker));
    }

    /// The hotkey: show the panel, or hide it when it already shows.
    pub fn toggle(&mut self, tx: &mut Tx) {
        let visible = self.me.is_visible();
        crate::trace::panel("toggle", visible, self.shown);
        if visible {
            self.dismiss(tx);
        } else {
            self.show(tx);
        }
    }

    /// The panel's top edge below the display's, for the configured position. Centering puts
    /// the field (not the whole panel, which grows with results) at the display's middle.
    fn top_margin(&self) -> f32 {
        match self.position {
            Position::Top => TOP_MARGIN,
            Position::Center => dupin_mac::window::pointer_display_height().map_or(TOP_MARGIN, |h| {
                ((h as f32 - crate::view::HEADER_H) / 2.0).max(0.0)
            }),
        }
    }

    fn show(&mut self, tx: &mut Tx) {
        crate::trace::summoned();
        if let Some(problem) = self.backdrop.refresh() {
            crate::log::warn(problem);
        }
        self.query.focused = true;
        self.me.place(
            DisplayTarget::UnderPointer,
            OverlayPlacement::Edge {
                edge: ScreenEdge::Top,
                margin: self.top_margin(),
                align: Align::Center,
                inset: 0.0,
            },
        );
        self.shown = true;
        self.shown_at = tx.clock();
        self.force_input_source();
        self.me.show();
    }

    fn force_input_source(&mut self) {
        use dupin_mac::input_source::{Target, select};
        if self.restore_input.is_some() {
            return;
        }
        let target = match &self.input_source {
            InputSource::Keep => return,
            InputSource::Ascii => Target::Ascii,
            InputSource::Id(id) => Target::Id(id.clone()),
        };
        self.restore_input = select(&target);
    }

    /// Hide and reset, as Esc and losing focus do.
    pub fn dismiss(&mut self, tx: &mut Tx) {
        let effects = self.session.reset();
        self.apply(effects, tx);
        self.hide(tx);
    }

    fn prepare_settings(&mut self) {
        crate::trace::settings("requested");
        self.settings_after_close = true;
    }

    pub fn closed(&mut self, _: &mut Tx) {
        crate::trace::panel("closed", self.me.is_visible(), self.shown);
        if std::mem::take(&mut self.snapshots_after_close) {
            self.snapshots.open();
        }
        if std::mem::take(&mut self.settings_after_close) {
            crate::trace::settings("panel closed");
            self.prefs.open();
        }
    }

    fn hide(&mut self, tx: &mut Tx) {
        crate::trace::panel("hide", self.me.is_visible(), self.shown);
        self.action_screen = None;
        if let Some(preview) = self.quick_look.borrow().as_ref() {
            preview.close();
        }
        if let Some(previous) = self.restore_input.take() {
            previous.restore();
        }
        self.query.clear();
        self.shown = false;
        self.notice = None;
        self.snapshot_notice_until = None;
        tx.close_window();
        if !self.settings_after_close
            && !self.snapshots_after_close
            && let Some(request) = self.relaunch.take()
        {
            self.relaunch_when_idle(request);
        }
    }

    pub fn show_notice(&mut self, message: String, tx: &mut Tx) {
        self.snapshot_notice_until = None;
        self.notice = Some(message);
        tx.instant();
        self.show(tx);
    }

    /// The field's text changed.
    pub fn edited(&mut self, tx: &mut Tx) {
        self.action_screen = None;
        crate::trace::edited(&self.query.text);
        self.typed_at = tx.clock();
        let effects = self.session.set_query(&self.query.text);
        self.apply(effects, tx);
    }

    /// Keep panel shortcuts available while the definition owns the text selection.
    pub fn unfocused_key(&mut self, ev: &KeyEvent, tx: &mut Tx) -> bool {
        if self.action_screen.is_some() {
            return self.key(ev, tx);
        }
        if tx.focused().is_some() {
            return false;
        }
        if ev.key == Key::Backspace && !self.query.text.is_empty() {
            tx.focus("query", false);
            self.query_edit().key(ev, None);
            self.edited(tx);
            return true;
        }
        if let Key::Text(text) = &ev.key
            && !(ev.mods.cmd || ev.mods.ctrl || ev.mods.alt)
        {
            tx.focus("query", false);
            self.query_edit().insert(text);
            self.edited(tx);
            return true;
        }
        self.key(ev, tx)
    }

    pub fn snapshot_notice(&mut self, message: String, tx: &mut Tx) {
        self.notice = Some(message);
        let due = tx.clock() + 5.0;
        self.snapshot_notice_until = Some(due);
        tx.wake_at(due);
    }

    pub fn open_snapshots(&mut self) {
        self.snapshots.open();
    }

    pub fn resume_snapshot(&mut self, saved: dupin_core::snapshots::Snapshot, tx: &mut Tx) {
        match self.session.restore(&saved) {
            Ok(effects) => {
                self.action_screen = None;
                self.apply(effects, tx);
                self.show(tx);
            }
            Err(problem) => self.snapshots.update(move |s, _| s.report(problem)),
        }
    }

    /// Keys the field sees first. True consumes the key.
    pub fn key(&mut self, ev: &KeyEvent, tx: &mut Tx) -> bool {
        if ev.mods.cmd && ev.mods.shift && matches!(ev.key, Key::Command('s') | Key::Command('S')) {
            match self.session.capture() {
                Ok(saved) => {
                    self.snapshots.update(move |s, _| s.save(saved));
                    self.snapshot_notice("Saving snapshot…".into(), tx);
                }
                Err(problem) => self.snapshot_notice(problem, tx),
            }
            return true;
        }
        if ev.mods.cmd && ev.mods.shift && matches!(ev.key, Key::Command('o') | Key::Command('O')) {
            self.snapshots_after_close = true;
            self.dismiss(tx);
            return true;
        }
        crate::trace::key(ev);
        if let Some(preview) = self.quick_look.borrow().as_ref() {
            preview.trace_picker_input();
        }
        match &ev.key {
            // An accessory app has no menu bar, so ⌘Q in the panel is the way to quit.
            Key::Command('q') => {
                self.updater.install_on_quit();
                tx.quit();
                return true;
            }
            Key::Command(',') => {
                self.prepare_settings();
                self.dismiss(tx);
                return true;
            }
            _ => {}
        }
        if let Some(screen) = &mut self.action_screen {
            match screen.key(&ev.key) {
                ActionKey::Back => {
                    self.action_screen = None;
                    tx.focus("query", false);
                }
                ActionKey::Run(key, action) => self.pick_action(&key, &action, tx),
                ActionKey::Stay => {
                    tx.scroll_into_view(format!("action-{}", screen.selected));
                }
            }
            return true;
        }
        if self.query.is_composing() {
            return false;
        }
        if is_quick_look_shortcut(&ev.key) {
            if !self
                .session
                .selected_item()
                .is_some_and(|item| item.item_action(dupin_mac::quick_look::ACTION_ID).is_some())
            {
                return false;
            }
            let key = self.session.result_key(self.session.selected());
            let effects = self
                .session
                .run_key_action(&key, dupin_mac::quick_look::ACTION_ID);
            self.apply(effects, tx);
            return true;
        }
        let m = ev.mods;
        let plain = !(m.cmd || m.ctrl || m.alt);
        let intent = match &ev.key {
            Key::Tab
                if plain
                    && !m.shift
                    && self.query.anchor == self.query.focus
                    && self.query.focus == self.query.text.len() =>
            {
                Intent::Complete
            }
            Key::Text(t) if t == " " && plain && !m.shift => Intent::Space,
            Key::Backspace if plain => Intent::Backspace,
            Key::Up => Intent::Up,
            Key::Down => Intent::Down,
            Key::Right
                if plain
                    && !m.shift
                    && self.query.anchor == self.query.focus
                    && self.query.focus == self.query.text.len() =>
            {
                Intent::Actions
            }
            Key::Command('k') => Intent::Actions,
            Key::Text(t) if m.ctrl && t.eq_ignore_ascii_case("k") => Intent::Actions,
            Key::Enter if m.cmd => Intent::OpenAlternate,
            Key::Enter => Intent::Open,
            Key::Escape => Intent::Dismiss,
            _ => return false,
        };
        let Some(effects) = self.session.intent(intent) else {
            return false;
        };
        self.apply(effects, tx);
        true
    }

    /// A click on a result row.
    pub fn open(&mut self, id: &str, tx: &mut Tx) {
        if self.session.select_key(id)
            && let Some(effects) = self.session.intent(Intent::Open)
        {
            self.apply(effects, tx);
        }
    }

    pub fn actions(&mut self, id: &str, tx: &mut Tx) {
        if self.session.select_key(id) {
            let effects = self.session.intent(Intent::Actions).unwrap_or_default();
            self.apply(effects, tx);
        }
    }

    pub fn pick_action(&mut self, key: &str, action: &str, tx: &mut Tx) {
        if !self
            .action_screen
            .as_ref()
            .is_some_and(|screen| screen.key == key)
        {
            return;
        }
        self.action_screen = None;
        let effects = self.session.run_key_action(key, action);
        self.apply(effects, tx);
    }

    fn deliver(&mut self, origin: CommandId, delivery: Delivery) {
        if self.session.deliver_from(origin.clone(), delivery) {
            if self
                .deadlines
                .get(&origin)
                .is_some_and(|(g, _)| !self.session.pending(&origin, *g))
            {
                self.deadlines.remove(&origin);
            }
            if self
                .action_screen
                .as_ref()
                .is_some_and(|screen| !screen.is_current(&self.session))
            {
                self.action_screen = None;
            }
            self.fetch_icons();
        }
    }

    fn refresh_apps(&mut self, tx: &mut Tx) {
        if self.session.command_id().as_str() == crate::commands::APPS {
            let effects = self.session.refresh();
            self.apply(effects, tx);
        }
    }

    fn apply(&mut self, effects: Vec<Effect>, tx: &mut Tx) {
        for effect in effects {
            match effect {
                Effect::ShowActions { id, generation: _ } => {
                    let Some(item) = self
                        .session
                        .items()
                        .iter()
                        .enumerate()
                        .find_map(|(i, item)| (self.session.result_key(i) == id).then_some(item))
                    else {
                        continue;
                    };
                    self.action_screen = Some(ActionScreen {
                        key: id,
                        item: item.clone(),
                        layout_key: self.session.layout_key(self.session.selected()),
                        selected: 0,
                    });
                }
                Effect::Search {
                    command,
                    generation,
                    query,
                } => {
                    self.action_screen = None;
                    let spec = self.session.provider_spec(&command).expect("registered provider");
                    let limit = spec.candidate_limit();
                    let due = tx.clock() + spec.deadline.as_secs_f64();
                    self.deadlines.retain(|id, (g, _)| {
                        let keep = self.session.pending(id, *g);
                        if !keep && let Some(worker) = self.searchers.get(id) {
                            worker.cancel();
                        }
                        keep
                    });
                    for (id, worker) in &self.searchers {
                        if !self.session.pending(id, generation) {
                            worker.cancel();
                        }
                    }
                    if let Some(searcher) = self.searchers.get(&command) {
                        searcher.search(generation, query, limit);
                        self.deadlines.insert(command, (generation, due));
                        tx.wake_at(due);
                    }
                }
                Effect::CancelSearch => {
                    self.action_screen = None;
                    self.searchers.values().for_each(Searcher::cancel);
                    self.deadlines.clear();
                }
                Effect::ClearQuery => self.query.clear(),
                Effect::ReplaceQuery(text) => self.query.replace_text(text),
                Effect::SaveHistory => self.saver.save(self.session.history()),
                Effect::Run(action) => match action {
                    dupin_core::Action::Copy(text) => tx.copy(text),
                    dupin_core::Action::OpenPreferences => self.prepare_settings(),
                    dupin_core::Action::Preview(path) => {
                        let shown = self
                            .quick_look
                            .borrow()
                            .as_ref()
                            .is_some_and(|preview| preview.show(&path));
                        if !shown {
                            crate::log::warn("couldn't show the Quick Look preview");
                        }
                    }
                    dupin_core::Action::Open(path) => {
                        self.run_workspace_action(dupin_mac::workspace::Action::Open(path))
                    }
                    dupin_core::Action::Reveal(path) => {
                        self.run_workspace_action(dupin_mac::workspace::Action::Reveal(path))
                    }
                    dupin_core::Action::OpenUrl(url) => {
                        self.run_workspace_action(dupin_mac::workspace::Action::OpenUrl(url))
                    }
                },
                Effect::Hide => self.hide(tx),
                Effect::RevealSelection => {
                    if self.session.selected_item().is_some() {
                        tx.scroll_into_view(crate::view::row_key(
                            &self.session.layout_key(self.session.selected()),
                        ));
                    }
                }
            }
        }
        mark_query_qualifiers(&mut self.query, self.session.command_id());
    }

    fn run_workspace_action(&self, action: dupin_mac::workspace::Action) {
        let running = self.running.clone();
        running.fetch_add(1, Ordering::AcqRel);
        std::thread::spawn(move || {
            if !dupin_mac::workspace::run(&action) {
                crate::log::warn(&format!("the system couldn't carry out {action:?}"));
            }
            running.fetch_sub(1, Ordering::AcqRel);
        });
    }

    pub fn preview_focus_is_active(&self) -> bool {
        self.quick_look
            .borrow()
            .as_ref()
            .is_some_and(|preview| preview.panel_is_key() || preview.owner_is_key())
    }

    pub fn preview_panel_resigned(&mut self, tx: &mut Tx) {
        if !self.shown {
            return;
        }
        let keep = self
            .quick_look
            .borrow()
            .as_ref()
            .is_some_and(|preview| preview.finish_focus_transfer());
        if !keep {
            self.dismiss(tx);
        }
    }

    pub fn poll(&mut self, clock: f64, tx: &mut Tx) -> bool {
        let due = take_due(&mut self.deadlines, clock);
        let mut changed = false;
        if self.snapshot_notice_until.is_some_and(|due| clock >= due) {
            self.snapshot_notice_until = None;
            self.notice = None;
            changed = true;
        }
        for (id, generation) in due {
            if self.session.pending(&id, generation) {
                if let Some(worker) = self.searchers.get(&id) {
                    worker.cancel();
                }
                changed |= self.session.deliver_from(id, Delivery::Expired { generation });
            }
        }
        if let Some(next) = self
            .deadlines
            .values()
            .map(|(_, due)| *due)
            .min_by(f64::total_cmp)
        {
            tx.wake_at(next);
        }
        changed
    }

    /// Ask for the icons of the rows on screen. Handles live in `icons`, which the cache
    /// needs to keep them, and which is trimmed to the current rows when it grows too big.
    fn fetch_icons(&mut self) {
        let Some(cache) = &self.icon_cache else { return };
        if self.icons.len() > ICON_CAP {
            let current: std::collections::HashSet<String> = self
                .session
                .items()
                .iter()
                .enumerate()
                .map(|(i, _)| self.session.layout_key(i))
                .collect();
            self.icons.retain(|id, _| current.contains(id.as_str()));
        }
        for (index, item) in self.session.items().iter().enumerate() {
            if let Some(path) = &item.icon
                && !self.icons.contains_key(&self.session.layout_key(index))
            {
                self.icons
                    .insert(self.session.layout_key(index), cache.app_icon(path, ICON_SIZE));
            }
        }
    }
}

/// The backdrop `pick` names, logging why it can't be used.
fn backdrop_source(pick: &Backdrop) -> crate::shader::Source {
    let source = crate::shader::Source::new(pick);
    if let Some(problem) = source.problem() {
        crate::log::warn(problem);
    }
    source
}

fn reconcile_snapshot(
    snapshot: Result<Config, String>,
    acknowledged: &Config,
    save_failed: bool,
) -> (Result<Config, String>, Option<String>) {
    match snapshot {
        Err(error) if save_failed => (Ok(acknowledged.clone()), Some(error)),
        snapshot => (snapshot, None),
    }
}

fn accept_config(
    config: Config,
    previous: &Config,
    install: impl FnOnce(Option<&rune::Shortcut>) -> Result<(), rune::ShortcutError>,
) -> Result<Config, String> {
    let shortcut = crate::hotkey::parse(&config.hotkey).map_err(|_| {
        "The hotkey in config.toml is invalid; keeping the previous configuration.".to_owned()
    })?;
    if let Err(error) = install(shortcut.as_ref())
        && crate::hotkey::parse(&previous.hotkey).ok().as_ref() != Some(&shortcut)
    {
        return Err(format!(
            "Couldn't apply the hotkey from config.toml: {error}; keeping the previous configuration."
        ));
    }
    Ok(config)
}

fn is_quick_look_shortcut(key: &Key) -> bool {
    matches!(key, Key::Command('y'))
}

fn take_due(deadlines: &mut HashMap<CommandId, (u64, f64)>, clock: f64) -> Vec<(CommandId, u64)> {
    let due: Vec<_> = deadlines
        .iter()
        .filter(|(_, (_, deadline))| clock >= *deadline)
        .map(|(id, (generation, _))| (id.clone(), *generation))
        .collect();
    for (id, _) in &due {
        deadlines.remove(id);
    }
    due
}

#[cfg(test)]
mod ui_helper_tests {
    use super::*;

    #[test]
    fn quick_look_shortcut_is_command_y() {
        assert!(is_quick_look_shortcut(&Key::Command('y')));
        assert!(!is_quick_look_shortcut(&Key::Command('k')));
        assert!(!is_quick_look_shortcut(&Key::Text("y".into())));
    }

    #[test]
    fn controlled_clock_expires_only_latest_deadline_once() {
        let id = CommandId::new("sample.search").unwrap();
        let mut deadlines = HashMap::new();
        deadlines.insert(id.clone(), (1, 10.0));
        deadlines.insert(id.clone(), (2, 20.0));
        assert!(take_due(&mut deadlines, 10.0).is_empty());
        assert_eq!(take_due(&mut deadlines, 20.0), [(id.clone(), 2)]);
        assert!(take_due(&mut deadlines, 21.0).is_empty());
        deadlines.insert(id.clone(), (3, 30.0));
        deadlines.remove(&id);
        assert!(take_due(&mut deadlines, 30.0).is_empty());
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;

    #[test]
    fn a_failed_save_and_invalid_snapshot_restore_acknowledged_values() {
        let acknowledged = Config::default();
        let (restored, problem) = reconcile_snapshot(Err("invalid TOML".into()), &acknowledged, true);
        assert_eq!(restored, Ok(acknowledged.clone()));
        assert_eq!(problem.as_deref(), Some("invalid TOML"));
        let disk = Config {
            file_limit: 30,
            ..acknowledged.clone()
        };
        assert_eq!(
            reconcile_snapshot(Ok(disk.clone()), &acknowledged, true),
            (Ok(disk), None)
        );
        assert!(
            reconcile_snapshot(Err("invalid TOML".into()), &acknowledged, false)
                .0
                .is_err()
        );
    }

    #[test]
    fn an_unavailable_unchanged_hotkey_does_not_reject_other_settings() {
        let previous = Config::default();
        let changed = Config {
            file_limit: 50,
            ..previous.clone()
        };
        let accepted = accept_config(changed.clone(), &previous, |_| Err(rune::ShortcutError::Conflict));
        assert_eq!(accepted, Ok(changed));
    }

    #[test]
    fn an_unavailable_changed_hotkey_keeps_the_previous_config() {
        let previous = Config::default();
        let changed = Config {
            hotkey: "ctrl+alt+k".into(),
            file_limit: 50,
            ..previous.clone()
        };
        assert!(accept_config(changed, &previous, |_| Err(rune::ShortcutError::Conflict)).is_err());
    }
}

#[cfg(test)]
mod qualifier_tests {
    use super::*;

    fn files() -> CommandId {
        CommandId::new(crate::commands::FILES).unwrap()
    }

    #[test]
    fn inserting_a_qualifier_before_text_keeps_the_caret_inside_the_unfinished_token() {
        let mut edit = TextEdit::new("report");
        edit.set_caret(0);
        edit.insert("in:/");
        mark_query_qualifiers(&mut edit, &files());
        assert_eq!(edit.text, "in:/report");
        assert_eq!(edit.focus, 4);
        assert_eq!(edit.pills().count(), 0);
        edit.insert("tmp ");
        mark_query_qualifiers(&mut edit, &files());
        assert_eq!(edit.text, "in:/tmp report");
        assert_eq!(edit.focus, 8);
        assert_eq!(edit.pills().count(), 1);
    }

    #[test]
    fn moving_the_caret_outside_a_token_allows_idempotent_marking() {
        let mut edit = TextEdit::new("in:/tmp report");
        edit.set_caret(4);
        mark_query_qualifiers(&mut edit, &files());
        assert_eq!(edit.focus, 4);
        assert_eq!(edit.pills().count(), 0);
        edit.set_caret(edit.text.len());
        mark_query_qualifiers(&mut edit, &files());
        assert_eq!(edit.pills().count(), 1);
        let before = edit.focus;
        mark_query_qualifiers(&mut edit, &files());
        assert_eq!(edit.focus, before);
        assert_eq!(edit.text, "in:/tmp report");
    }

    #[test]
    fn qualifier_pills_keep_raw_unicode_text_and_delete_as_whole_tokens() {
        let raw = "日本語 kind:pdf";
        let mut edit = TextEdit::new(raw);
        mark_query_qualifiers(&mut edit, &files());
        assert_eq!(edit.text, raw);
        let (range, _) = edit.pills().next().unwrap();
        assert_eq!(&edit.text[range], "kind:pdf");
        edit.set_caret(edit.text.len());
        edit.key(
            &KeyEvent {
                key: Key::Backspace,
                mods: Default::default(),
            },
            None,
        );
        mark_query_qualifiers(&mut edit, &files());
        assert_eq!(edit.text, "日本語 ");
        assert_eq!(edit.pills().count(), 0);
        edit.undo();
        mark_query_qualifiers(&mut edit, &files());
        assert_eq!(edit.text, raw);
        assert_eq!(edit.pills().count(), 1);
    }

    #[test]
    fn query_replacement_and_mode_changes_recompute_marks_without_changing_text() {
        let mut edit = TextEdit::new("kind:pdf");
        mark_query_qualifiers(&mut edit, &files());
        assert_eq!(edit.pills().count(), 1);
        edit.replace_text("report ext:rs");
        mark_query_qualifiers(&mut edit, &files());
        assert_eq!(edit.pills().count(), 1);
        let (range, _) = edit.pills().next().unwrap();
        assert_eq!(&edit.text[range], "ext:rs");
        mark_query_qualifiers(&mut edit, &CommandId::new(crate::commands::APPS).unwrap());
        assert_eq!(edit.text, "report ext:rs");
        assert_eq!(edit.pills().count(), 0);
    }
}

#[cfg(test)]
mod action_screen_tests {
    use super::*;
    use dupin_core::{Action, CommandRegistry, CommandSpec, Item, QueryPresentation};
    use std::path::PathBuf;

    fn fixture() -> (Session, ActionScreen) {
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
                items: vec![Item::path(
                    PathBuf::from("/fixture/example"),
                    "Example".into(),
                    vec![],
                )],
            },
        );
        session.intent(Intent::Actions);
        let screen = ActionScreen {
            key: session.result_key(0),
            layout_key: session.layout_key(0),
            item: session.items()[0].clone(),
            selected: 0,
        };
        (session, screen)
    }

    #[test]
    fn keyboard_navigation_is_bounded_and_runs_the_captured_key() {
        let (_, mut screen) = fixture();
        screen.key(&Key::Up);
        assert_eq!(screen.selected, 0);
        for _ in 0..10 {
            screen.key(&Key::Down);
        }
        assert_eq!(screen.selected, 2);
        screen.key(&Key::Up);
        assert_eq!(
            screen.key(&Key::Enter),
            ActionKey::Run(screen.key.clone(), "reveal".into())
        );
        assert_eq!(screen.key(&Key::Text("x".into())), ActionKey::Stay);
    }

    #[test]
    fn cancel_returns_without_changing_search_or_selection() {
        let (session, mut screen) = fixture();
        let before = (
            session.query().to_owned(),
            session.selected(),
            session.result_key(0),
        );
        for key in [Key::Left, Key::Escape] {
            assert_eq!(screen.key(&key), ActionKey::Back);
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
        screen.selected = 2;
        let ActionKey::Run(key, action) = screen.key(&Key::Enter) else {
            panic!("expected action")
        };
        let effects = session.run_key_action(&key, &action);
        assert!(
            effects.iter().any(
                |effect| matches!(effect, Effect::Run(Action::Copy(value)) if value == "/fixture/example")
            )
        );
    }
}
