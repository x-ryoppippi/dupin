//! The panel's state: a `Session` from dupin-core plus what only the UI needs (the text
//! field, icons, a notice). Handlers translate keys into session intents and carry out the
//! effects the session returns.

mod action_screen;
mod calendar;
mod clipboard;
mod control;
mod effects;
mod extensions;
mod files;
mod hotkeys;
mod keys;
mod lifecycle;
mod notices;
mod query;
mod reload;
mod scripts;
mod search;
mod selection;
mod snapshots;
mod snippets;
mod thumbnails;
mod window_management;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, RwLock};

use dupin_core::window_management::Layout as WindowLayout;
use dupin_core::{
    Backdrop, CommandId, Config, DisplaySelection, History, InputSource, Position, Searcher, Session,
};
use dupin_mac::{AppsProvider, AppsWatcher, quick_look::Previewer, window::DisplayNotice};
use rune::{AppHandle, Image, ImageCache, TextEdit, Tx};

use crate::scripts::catalog::Catalog;

pub use action_screen::{ActionStage, ApplicationState, SEND_TO};
pub use control::pick_indices;
pub use lifecycle::Busy;
pub use search::ICON_SIZE;
pub use thumbnails::PREVIEW_HEIGHT;

use action_screen::ActionScreen;
use control::{CallerHold, Picker};
pub(crate) use extensions::Extensions;
use lifecycle::Restart;
use notices::Notices;
use reload::backdrop_source;
use search::spawn_searcher;
use selection::SelectionRequest;
use snippets::SnippetRun;

enum PendingPaste {
    Snippet(SnippetRun),
    /// What an extension's action just put on the clipboard, pasted only while the permit to
    /// paste for it still holds.
    Clipboard {
        permit: dupin_extension_bridge::Permit,
    },
    /// An extension row's text, pasted only while the permit to paste for it still holds.
    Extension {
        text: String,
        permit: dupin_extension_bridge::Permit,
    },
}

pub struct Windows {
    pub panel: AppHandle<Ui>,
    pub prefs: AppHandle<crate::prefs::Prefs>,
    pub snapshots: AppHandle<crate::snapshots::Snapshots>,
    pub scripts: AppHandle<crate::scripts::output::Output>,
}

pub struct Configuration {
    pub values: Rc<RefCell<Config>>,
    pub locked_keys: crate::settings::LockedKeys,
    pub hotkey: Arc<crate::hotkey::Binding>,
    pub files: Arc<crate::settings::Service>,
}

pub struct Ui {
    pub picker: Option<Picker>,
    caller_hold: CallerHold,
    control_save: Option<(u64, crate::control::Call)>,
    pub theme: dupin_core::config::Theme,
    pub active_radius: f32,
    pub script_entry: Option<crate::scripts::entry::Entry>,
    pub script_card: Option<Arc<crate::scripts::runner::Snapshot>>,
    scripts: Arc<RwLock<Catalog>>,
    scripts_catalog_ready: bool,
    runner: crate::scripts::runner::Runner,
    _scripts_watcher: Option<crate::catalog_watch::Watch>,
    pub action_screen: Option<ActionScreen>,
    next_lookup: u64,
    notices: Notices,
    pub session: Session,
    pub query: TextEdit,
    pub config_problem: Option<String>,
    pub hotkey_problem: Option<String>,
    pub registration_notice: Option<String>,
    pub selection_permission_required: bool,
    selection_request: SelectionRequest,
    configuration: Configuration,
    quick_look: Rc<RefCell<Option<Previewer>>>,
    acknowledged_config: Config,
    bindings_applied: bool,
    pub icons: HashMap<dupin_core::Icon, Image>,
    /// The images extension rows name, made off the panel's thread.
    pub(crate) thumbnails: thumbnails::Thumbnails,
    /// The rows the windowed result list last showed.
    visible_rows: std::ops::Range<usize>,
    /// Between `show` and `hide`: the backdrop animates only then.
    pub shown: bool,
    /// The clock when the panel last showed, and when the query last changed.
    pub shown_at: f64,
    pub typed_at: f64,
    pub backdrop: crate::shader::Source,
    backdrop_pick: Backdrop,
    icon_cache: Option<ImageCache>,
    searchers: HashMap<CommandId, Searcher>,
    pub(crate) currency: crate::calculator::Currency,
    pub(crate) web: Arc<RwLock<crate::web::WebCatalog>>,
    pub(crate) snippets: Arc<RwLock<dupin_core::snippets::SnippetCatalog>>,
    /// What to paste once the panel has closed and the front app has its keys back.
    paste_after_close: Option<PendingPaste>,
    /// Why something wasn't pasted, to show once the panel has closed.
    notice_after_close: Option<String>,
    sharing: dupin_mac::sharing::Picker,
    panel_window: Option<rune::WindowHandle>,
    pub(crate) file_demotions: Arc<RwLock<Vec<String>>>,
    pub(crate) system_provider: Arc<dupin_mac::system_commands::SystemProvider>,
    deadlines: HashMap<CommandId, (u64, f64)>,
    extensions: Extensions,
    calendar_access: dupin_mac::privacy::CalendarAccess,
    _apps_watcher: Option<AppsWatcher>,
    pub(crate) clipboard: crate::clipboard::Clipboard,
    saver: crate::store::Saver,
    position: Position,
    display: DisplaySelection,
    pub display_notice: Option<DisplayNotice>,
    input_source: InputSource,
    /// The input source to put back when the panel hides, if showing it switched one.
    restore_input: Option<dupin_mac::input_source::Previous>,
    /// An update was staged while the panel showed: restart into it when the panel hides.
    relaunch: Option<Restart>,
    settings_after_close: bool,
    snapshots_after_close: bool,
    accessibility_after_close: bool,
    pub(crate) window_management_enabled: bool,
    frontmost_pid: Option<i32>,
    window_action_after_close: Option<(WindowLayout, Option<i32>, Busy)>,
    window_action_running: bool,
    pub window_permission_required: bool,
    system_runner: dupin_mac::system_commands::Runner,
    /// Native actions that haven't returned yet; a relaunch waits for them.
    running: Arc<AtomicUsize>,
    file_transfers: dupin_mac::file_ops::FileTransfers,
    updater: crate::update::Updater,
    me: AppHandle<Ui>,
    prefs: AppHandle<crate::prefs::Prefs>,
    snapshots: AppHandle<crate::snapshots::Snapshots>,
    script_output: AppHandle<crate::scripts::output::Output>,
    script_output_after_close: Option<crate::scripts::runner::RunId>,
}

impl Ui {
    pub fn new(
        windows: Windows,
        updater: crate::update::Updater,
        configuration: Configuration,
        history: History,
        calendar_access: dupin_mac::privacy::CalendarAccess,
        quick_look: Rc<RefCell<Option<Previewer>>>,
        running: Arc<AtomicUsize>,
    ) -> Ui {
        let Windows {
            panel: me,
            prefs,
            snapshots,
            scripts: script_output,
        } = windows;
        let config = configuration.values.borrow().clone();
        let apps_provider = Arc::new(AppsProvider::standard());
        let currency = crate::calculator::Currency::start({
            let me = me.clone();
            move || {
                me.update(|s, tx| {
                    if s.currency.enabled() && !s.session.query().trim().is_empty() {
                        s.refresh_default(tx);
                    }
                })
            }
        });
        currency.set_enabled(config.calculator && config.calculator_currency);
        let clipboard = crate::clipboard::Clipboard::new(&config.clipboard);
        let registration = crate::commands::register(
            &config,
            apps_provider.clone(),
            history,
            (!rune::tooling::is_headless()).then(|| calendar_access.clone()),
            currency.calculator(),
        )
        .expect("built-in command registration is valid");
        let runner = crate::scripts::runner::Runner::new(
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_default(),
            Arc::new({
                let me = me.clone();
                move |id, finished| me.update_if(move |s, tx| s.script_changed(id, finished, tx))
            }),
        );
        let scripts_watcher = crate::scripts::watch(me.clone());
        let bridge = crate::extensions::access::open(crate::extensions::access::Platform {
            scripts: registration.scripts.clone(),
            runner: runner.clone(),
            calendar_access: calendar_access.clone(),
            clipboard: clipboard.settings(),
        });
        clipboard.start(&bridge);
        bridge.set_listener({
            let me = me.clone();
            move |notice| me.update(move |ui, tx| ui.extension_access_changed(notice, tx))
        });
        crate::extensions::access::refresh_os_access(&bridge, &calendar_access);
        let apps_watcher = apps_provider.watch({
            let me = me.clone();
            move || me.update(|s, tx| s.refresh_default(tx))
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

        let backdrop = backdrop_source(&config.backdrop);
        let searchers = registration
            .providers
            .into_iter()
            .map(|(id, provider)| (id.clone(), spawn_searcher(me.clone(), id, provider)))
            .collect();
        let sources = registration.session.command_ids();
        snapshots.update(move |s, _| s.set_sources(sources));
        me.update(|s, tx| s.refresh_default(tx));
        let mut query = TextEdit::new("");
        query.focused = true;
        let mut ui = Ui {
            script_output,
            script_output_after_close: None,
            script_entry: None,
            script_card: None,
            scripts: registration.scripts,
            scripts_catalog_ready: scripts_watcher.is_none(),
            runner,
            _scripts_watcher: scripts_watcher,
            picker: None,
            control_save: None,
            theme: config.theme.clone(),
            active_radius: config.theme.corner_radius as f32,
            action_screen: None,
            next_lookup: 0,
            notices: Notices::default(),
            acknowledged_config: config.clone(),
            bindings_applied: false,
            session: registration.session,
            saver: crate::store::Saver::spawn(),
            query,
            config_problem: None,
            hotkey_problem: None,
            registration_notice: registration.notice,
            selection_permission_required: false,
            selection_request: SelectionRequest::Idle,
            icons: HashMap::new(),
            thumbnails: thumbnails::Thumbnails::new(),
            visible_rows: 0..0,
            shown: false,
            shown_at: 0.0,
            caller_hold: CallerHold::default(),
            typed_at: f64::NEG_INFINITY,
            backdrop,
            backdrop_pick: config.backdrop.clone(),
            icon_cache: None,
            searchers,
            currency,
            web: registration.web,
            snippets: registration.snippets,
            paste_after_close: None,
            notice_after_close: None,
            sharing: dupin_mac::sharing::Picker::default(),
            panel_window: None,
            file_demotions: registration.file_demotions,
            system_provider: registration.system_provider,
            deadlines: HashMap::new(),
            extensions: Extensions::new(bridge, registration.ports),
            calendar_access,
            _apps_watcher: apps_watcher,
            clipboard,
            position: config.position,
            display: config.display,
            display_notice: None,
            input_source: config.input_source.clone(),
            configuration,
            restore_input: None,
            relaunch: None,
            settings_after_close: false,
            snapshots_after_close: false,
            accessibility_after_close: false,
            window_management_enabled: config.window_management,
            frontmost_pid: None,
            window_action_after_close: None,
            window_action_running: false,
            window_permission_required: false,
            system_runner: if cfg!(test) || rune::tooling::is_headless() {
                |_| Err("System commands are unavailable in headless runs.".into())
            } else {
                dupin_mac::system_commands::run
            },
            running,
            file_transfers: dupin_mac::file_ops::FileTransfers::default(),
            updater,
            quick_look,
            me,
            prefs,
            snapshots,
        };
        ui.watch_extensions(config.extensions);
        ui
    }

    pub fn poll(&mut self, clock: f64, tx: &mut Tx) -> bool {
        self.poll_picker(clock, tx);
        let notice_expired = self.notices.expire(clock);
        self.poll_searches(clock, tx) || notice_expired
    }
}
