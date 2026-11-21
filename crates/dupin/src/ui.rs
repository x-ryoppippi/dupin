//! The panel's state: a `Session` from dupin-core plus what only the UI needs (the text
//! field, icons, a notice). Handlers translate keys into session intents and carry out the
//! effects the session returns.

use std::collections::HashMap;
use std::sync::Arc;

use dupin_core::{Config, Delivery, Effect, History, InputSource, Intent, Mode, Position, Searcher, Session};
use dupin_mac::{AppsProvider, SpotlightProvider, Target};
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
    icon_cache: Option<ImageCache>,
    apps: Searcher,
    files: Searcher,
    contents: Searcher,
    file_limit: usize,
    saver: crate::store::Saver,
    position: Position,
    input_source: InputSource,
    /// The input source to put back when the panel hides, if showing it switched one.
    restore_input: Option<dupin_mac::input_source::Previous>,
    me: AppHandle<Ui>,
    prefs: AppHandle<crate::prefs::Prefs>,
}

impl Ui {
    pub fn new(
        me: AppHandle<Ui>,
        prefs: AppHandle<crate::prefs::Prefs>,
        config: Config,
        history: History,
        notice: Option<String>,
    ) -> Ui {
        let apps_provider = Arc::new(AppsProvider::standard());
        // Scan before the first keystroke so it doesn't wait for the disk.
        let warm = apps_provider.clone();
        std::thread::spawn(move || warm.warm());
        let deliver = |me: AppHandle<Ui>| move |d: Delivery| me.update(move |s, _| s.deliver(d));
        let mut query = TextEdit::new("");
        query.focused = true;
        Ui {
            session: Session::new(history),
            saver: crate::store::Saver::spawn(),
            query,
            notice,
            icons: HashMap::new(),
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
            file_limit: config.file_limit,
            position: config.position,
            input_source: config.input_source,
            restore_input: None,
            me,
            prefs,
        }
    }

    /// Settings changed in the settings window. The hotkey is the settings window's to
    /// re-register; the rest takes effect from the next summon.
    pub fn apply_config(&mut self, config: &Config) {
        self.file_limit = config.file_limit;
        self.position = config.position;
        self.input_source = config.input_source.clone();
    }

    pub fn setup(s: &mut Ui, waker: Waker) {
        s.icon_cache = Some(ImageCache::new(waker));
    }

    /// The hotkey: show the panel, or hide it when it already shows.
    pub fn toggle(&mut self, tx: &mut Tx) {
        if self.me.is_visible() {
            self.dismiss(tx);
        } else {
            self.show();
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

    fn show(&mut self) {
        crate::trace::summoned();
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
        self.notice = None;
        tx.close_window();
    }

    pub fn show_notice(&mut self, message: String, tx: &mut Tx) {
        self.notice = Some(message);
        tx.instant();
        self.show();
    }

    /// The field's text changed.
    pub fn edited(&mut self, tx: &mut Tx) {
        let effects = self.session.set_query(&self.query.text);
        self.apply(effects, tx);
    }

    /// Keys the field sees first. True consumes the key.
    pub fn key(&mut self, ev: &KeyEvent, tx: &mut Tx) -> bool {
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
                    let limit = if mode == Mode::Apps {
                        APP_LIMIT
                    } else {
                        self.file_limit
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
                    std::thread::spawn(move || {
                        if !dupin_mac::workspace::run(&action) {
                            crate::log::warn(&format!("the system couldn't carry out {action:?}"));
                        }
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

    fn searchers(&self) -> [(Mode, &Searcher); 3] {
        [
            (Mode::Apps, &self.apps),
            (Mode::Files, &self.files),
            (Mode::Contents, &self.contents),
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
