//! The panel's state: a `Session` from dupin-core plus what only the UI needs (the text
//! field, icons, a notice). Handlers translate keys into session intents and carry out the
//! effects the session returns.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dupin_core::{
    Backdrop, Config, Delivery, Effect, History, InputSource, Intent, Mode, Position, Searcher, Session,
};
use dupin_mac::{AppsProvider, DictionaryProvider, SpotlightProvider, Target};
use rune::{
    Align, AppHandle, DisplayTarget, Image, ImageCache, Key, KeyEvent, OverlayPlacement, ScreenEdge,
    TextEdit, Tx, Waker,
};

/// The most apps a search returns; the list shows a few rows and scrolls.
const APP_LIMIT: usize = 50;
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
    pub icons: HashMap<String, Image>,
    /// Between `show` and `hide`: the backdrop animates only then.
    pub shown: bool,
    /// The clock when the panel last showed, and when the query last changed.
    pub shown_at: f64,
    pub typed_at: f64,
    pub backdrop: crate::shader::Source,
    backdrop_pick: Backdrop,
    icon_cache: Option<ImageCache>,
    apps: Searcher,
    files: Searcher,
    contents: Searcher,
    dictionary: Searcher,
    file_limit: usize,
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
        // Scan before the first keystroke so it doesn't wait for the disk.
        let warm = apps_provider.clone();
        std::thread::spawn(move || warm.warm());
        let deliver = |me: AppHandle<Ui>| move |d: Delivery| me.update(move |s, _| s.deliver(d));
        let (backdrop, problem) = crate::shader::Source::new(&config.backdrop);
        let notice = notice.or_else(|| problem.inspect(|p| crate::log::warn(p)));
        let mut query = TextEdit::new("");
        query.focused = true;
        Ui {
            session: Session::with_dictionary_keywords(history, config.dictionary_keywords.clone()),
            saver: crate::store::Saver::spawn(),
            query,
            notice,
            icons: HashMap::new(),
            shown: false,
            shown_at: 0.0,
            typed_at: f64::NEG_INFINITY,
            backdrop,
            backdrop_pick: config.backdrop.clone(),
            icon_cache: None,
            apps: Searcher::spawn("apps", apps_provider, deliver(me.clone())),
            files: Searcher::spawn(
                "files",
                Arc::new(SpotlightProvider::new(Target::Names)),
                deliver(me.clone()),
            ),
            contents: Searcher::spawn(
                "contents",
                Arc::new(SpotlightProvider::new(Target::Contents)),
                deliver(me.clone()),
            ),
            dictionary: Searcher::spawn(
                "dictionary",
                Arc::new(DictionaryProvider::default()),
                deliver(me.clone()),
            ),
            file_limit: config.file_limit,
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
        self.file_limit = config.file_limit;
        self.session
            .set_dictionary_keywords(config.dictionary_keywords.clone());
        self.position = config.position;
        self.input_source = config.input_source.clone();
        self.updater.set_enabled(config.auto_update);
        if self.backdrop_pick != config.backdrop {
            let (backdrop, problem) = crate::shader::Source::new(&config.backdrop);
            self.backdrop = backdrop;
            self.backdrop_pick = config.backdrop.clone();
            self.complain(problem);
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

    fn complain(&mut self, problem: Option<String>) {
        if let Some(problem) = problem {
            crate::log::warn(&problem);
            self.notice = Some(problem);
        }
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
        let problem = self.backdrop.refresh();
        self.complain(problem);
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

    /// Keys the field sees first. True consumes the key.
    pub fn key(&mut self, ev: &KeyEvent, tx: &mut Tx) -> bool {
        crate::trace::key(ev);
        let m = ev.mods;
        let plain = !(m.cmd || m.ctrl || m.alt);
        let intent = match &ev.key {
            Key::Text(t) if t == " " && plain && !m.shift => Intent::Space,
            Key::Backspace if plain => Intent::Backspace,
            Key::Up => Intent::Up,
            Key::Down => Intent::Down,
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
        if self.session.select_id(id)
            && let Some(effects) = self.session.intent(Intent::Open)
        {
            self.apply(effects, tx);
        }
    }

    fn deliver(&mut self, delivery: Delivery) {
        if self.session.deliver(delivery) {
            self.fetch_icons();
        }
    }

    fn apply(&mut self, effects: Vec<Effect>, tx: &mut Tx) {
        for effect in effects {
            match effect {
                Effect::Search {
                    mode,
                    generation,
                    query,
                } => {
                    let limit = match mode {
                        Mode::Apps => APP_LIMIT,
                        Mode::Dictionary => 1,
                        Mode::Files | Mode::Contents => self.file_limit,
                    };
                    for (m, searcher) in self.searchers() {
                        if m == mode {
                            searcher.search(generation, query.clone(), limit);
                        } else {
                            searcher.cancel();
                        }
                    }
                }
                Effect::CancelSearch => self.searchers().iter().for_each(|(_, s)| s.cancel()),
                Effect::ClearQuery => self.query.clear(),
                Effect::SaveHistory => self.saver.save(self.session.history()),
                Effect::Run(action) => {
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
                    if let Some(item) = self.session.selected_item() {
                        tx.scroll_into_view(crate::view::row_key(&item.id));
                    }
                }
            }
        }
    }

    fn searchers(&self) -> [(Mode, &Searcher); 4] {
        [
            (Mode::Apps, &self.apps),
            (Mode::Files, &self.files),
            (Mode::Contents, &self.contents),
            (Mode::Dictionary, &self.dictionary),
        ]
    }

    /// Ask for the icons of the rows on screen. Handles live in `icons`, which the cache
    /// needs to keep them, and which is trimmed to the current rows when it grows too big.
    fn fetch_icons(&mut self) {
        let Some(cache) = &self.icon_cache else { return };
        if self.icons.len() > ICON_CAP {
            let current: std::collections::HashSet<&str> =
                self.session.items().iter().map(|i| i.id.as_str()).collect();
            self.icons.retain(|id, _| current.contains(id.as_str()));
        }
        for item in self.session.items() {
            if let Some(path) = &item.icon
                && !self.icons.contains_key(&item.id)
            {
                self.icons
                    .insert(item.id.clone(), cache.app_icon(path, ICON_SIZE));
            }
        }
    }
}
