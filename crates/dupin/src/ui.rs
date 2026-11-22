//! The panel's state: a `Session` from dupin-core plus what only the UI needs (the text
//! field, icons, a notice). Handlers translate keys into session intents and carry out the
//! effects the session returns.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dupin_core::{
    Backdrop, CommandId, Config, Delivery, Effect, History, InputSource, Intent, Position, Searcher, Session,
};
use dupin_mac::{AppsProvider, AppsWatcher};
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

pub struct Ui {
    pub session: Session,
    pub query: TextEdit,
    /// A problem to show above the results until the panel next hides (a hotkey that
    /// couldn't be registered, a broken config file).
    pub notice: Option<String>,
    pub registration_notice: Option<String>,
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
    deadlines: HashMap<CommandId, (u64, f64)>,
    _apps_watcher: Option<AppsWatcher>,
    saver: crate::store::Saver,
    position: Position,
    input_source: InputSource,
    /// The input source to put back when the panel hides, if showing it switched one.
    restore_input: Option<dupin_mac::input_source::Previous>,
    /// An update was staged while the panel showed: restart into it when the panel hides.
    relaunch: bool,
    /// Actions handed to Launch Services that haven't returned yet; a relaunch waits for them.
    running: Arc<AtomicUsize>,
    updater: crate::update::Updater,
    me: AppHandle<Ui>,
    prefs: AppHandle<crate::prefs::Prefs>,
}

impl Ui {
    pub fn new(
        me: AppHandle<Ui>,
        prefs: AppHandle<crate::prefs::Prefs>,
        updater: crate::update::Updater,
        config: Config,
        history: History,
        notice: Option<String>,
    ) -> Ui {
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
        let mut query = TextEdit::new("");
        query.focused = true;
        Ui {
            session: Session::new(history, registration.registry)
                .expect("built-in command registration is valid"),
            saver: crate::store::Saver::spawn(),
            query,
            notice,
            registration_notice: registration.notice,
            icons: HashMap::new(),
            shown: false,
            shown_at: 0.0,
            typed_at: f64::NEG_INFINITY,
            backdrop,
            backdrop_pick: config.backdrop.clone(),
            icon_cache: None,
            searchers,
            deadlines: HashMap::new(),
            _apps_watcher: apps_watcher,
            position: config.position,
            input_source: config.input_source,
            restore_input: None,
            relaunch: false,
            running: Arc::new(AtomicUsize::new(0)),
            updater,
            me,
            prefs,
        }
    }

    /// Settings changed in the settings window. The hotkey is the settings window's to
    /// re-register; the rest takes effect from the next summon.
    pub fn apply_config(&mut self, config: &Config) {
        let registration =
            crate::commands::configured(config).expect("built-in command registration is valid");
        self.session
            .set_commands(registration.registry)
            .expect("built-in command registration is valid");
        self.registration_notice = registration.notice;
        self.position = config.position;
        self.input_source = config.input_source.clone();
        self.updater.set_enabled(config.auto_update);
        if self.backdrop_pick != config.backdrop {
            self.backdrop = backdrop_source(&config.backdrop);
            self.backdrop_pick = config.backdrop.clone();
        }
    }

    /// An update is staged: restart into it now, or once neither the panel nor Settings shows.
    pub fn updated(&mut self) {
        self.restart_into_update(false);
    }

    /// Settings' "Restart to Update".
    pub fn restart_to_update(&mut self) {
        self.restart_into_update(true);
    }

    /// The decision is made here, on the UI thread, so it sees the windows as they are now.
    /// Unless asked, Settings showing defers the restart too: it would close under the user.
    fn restart_into_update(&mut self, asked: bool) {
        if self.me.is_visible() || (!asked && self.prefs.is_visible()) {
            self.relaunch = true;
        } else if self.running.load(Ordering::Acquire) > 0 {
            self.relaunch_when_idle(asked);
        } else {
            self.updater.restart();
        }
    }

    /// Ask again once every action already handed off has been carried out, so exiting
    /// doesn't cut short the app or file the user just opened. The panel may have been
    /// summoned meanwhile, so `restart_into_update` decides afresh.
    fn relaunch_when_idle(&self, asked: bool) {
        let running = self.running.clone();
        let me = self.me.clone();
        std::thread::spawn(move || {
            while running.load(Ordering::Acquire) > 0 {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            me.update(move |s, _| s.restart_into_update(asked));
        });
    }

    pub fn setup(s: &mut Ui, waker: Waker) {
        s.icon_cache = Some(ImageCache::new(waker));
    }

    /// The hotkey: show the panel, or hide it when it already shows.
    pub fn toggle(&mut self, tx: &mut Tx) {
        if self.me.is_visible() {
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

    fn hide(&mut self, tx: &mut Tx) {
        if let Some(previous) = self.restore_input.take() {
            previous.restore();
        }
        self.query.clear();
        self.shown = false;
        self.notice = None;
        tx.close_window();
        if std::mem::take(&mut self.relaunch) {
            self.relaunch_when_idle(false);
        }
    }

    pub fn show_notice(&mut self, message: String, tx: &mut Tx) {
        self.notice = Some(message);
        tx.instant();
        self.show(tx);
    }

    /// The field's text changed.
    pub fn edited(&mut self, tx: &mut Tx) {
        crate::trace::edited(&self.query.text);
        self.typed_at = tx.clock();
        let effects = self.session.set_query(&self.query.text);
        self.apply(effects, tx);
    }

    /// Keep panel shortcuts available while the definition owns the text selection.
    pub fn unfocused_key(&mut self, ev: &KeyEvent, tx: &mut Tx) -> bool {
        if tx.focused().is_some() {
            return false;
        }
        if ev.key == Key::Backspace && !self.query.text.is_empty() {
            tx.focus("query", false);
            self.query.key(ev, None);
            self.edited(tx);
            return true;
        }
        if let Key::Text(text) = &ev.key
            && !(ev.mods.cmd || ev.mods.ctrl || ev.mods.alt)
        {
            tx.focus("query", false);
            self.query.insert(text);
            self.edited(tx);
            return true;
        }
        self.key(ev, tx)
    }

    /// Keys the field sees first. True consumes the key.
    pub fn key(&mut self, ev: &KeyEvent, tx: &mut Tx) -> bool {
        crate::trace::key(ev);
        if self.query.is_composing() {
            return false;
        }
        let m = ev.mods;
        let plain = !(m.cmd || m.ctrl || m.alt);
        let intent = match &ev.key {
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
            Key::Enter if m.cmd => Intent::OpenAlternate,
            Key::Enter => Intent::Open,
            Key::Escape => Intent::Dismiss,
            // An accessory app has no menu bar, so ⌘Q in the panel is the way to quit.
            Key::Command('q') => {
                self.updater.install_on_quit();
                tx.quit();
                return true;
            }
            Key::Command(',') => {
                self.dismiss(tx);
                self.prefs.open();
                return true;
            }
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

    fn deliver(&mut self, origin: CommandId, delivery: Delivery) {
        if self.session.deliver_from(origin.clone(), delivery) {
            if self
                .deadlines
                .get(&origin)
                .is_some_and(|(g, _)| !self.session.pending(&origin, *g))
            {
                self.deadlines.remove(&origin);
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
                    let menu = rune::PopupMenu::new().items(
                        item.actions
                            .iter()
                            .map(|action| rune::PopupItem::new(action.id.clone(), action.title.clone())),
                    );
                    tx.popup_menu(menu, "query", move |s: &mut Ui, picked, tx| {
                        if let Some(action_id) = picked {
                            let effects = s.session.run_key_action(&id, action_id);
                            s.apply(effects, tx);
                        }
                    });
                }
                Effect::Search {
                    command,
                    generation,
                    query,
                } => {
                    let spec = self.session.provider_spec(&command).expect("registered provider");
                    let limit = spec.result_limit;
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
                    self.searchers.values().for_each(Searcher::cancel);
                    self.deadlines.clear();
                }
                Effect::ClearQuery => self.query.clear(),
                Effect::ReplaceQuery(text) => self.query.replace_text(text),
                Effect::SaveHistory => self.saver.save(self.session.history()),
                Effect::Run(action) => {
                    if let dupin_core::Action::Copy(text) = action {
                        tx.copy(text);
                        continue;
                    }
                    // Launch Services can take a moment to answer; don't hold the frame for it.
                    let running = self.running.clone();
                    running.fetch_add(1, Ordering::AcqRel);
                    std::thread::spawn(move || {
                        if !dupin_mac::workspace::run(&action) {
                            crate::log::warn(&format!("the system couldn't carry out {action:?}"));
                        }
                        running.fetch_sub(1, Ordering::AcqRel);
                    });
                }
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
    }

    pub fn poll(&mut self, clock: f64, tx: &mut Tx) -> bool {
        let due = take_due(&mut self.deadlines, clock);
        let mut changed = false;
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
mod deadline_tests {
    use super::*;
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
