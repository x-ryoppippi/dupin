//! The settings window (⌘, in the panel): a sidebar of pages, each a card of settings, drawn
//! in OpenCode's theme as Hex's settings are. A release build adds a page for its updates.
//! Changes apply to the panel at once and are written back to config.toml. A read-only
//! config.toml (one Nix manages) shows its values, locked.

mod appearance;
mod controls;
mod extensions;
mod general;
mod keywords;
mod permissions;
mod search;
mod shortcut;
mod snippets;
mod sync;
mod updates;
pub mod welcome;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use dupin_core::Config;
use dupin_mac::privacy::{CalendarAccess, PermissionStatus};
use rune::{AppHandle, Cx, El, Icon, ScrollSpec, Shortcut, ShortcutError, Tx, col, el, icon, row, text};
use rune_kit::Kit;

use self::controls::{DESCRIPTION, ROW_PX};
use self::general::LoginPreference;
use self::shortcut::ShortcutState;
use crate::hotkey::{Binding, LauncherShortcut};
use crate::icons;
use crate::ui::Ui;
use crate::update::Updater;

/// The window's first size, before AppKit restores the one the user left it at.
pub const SIZE: (f32, f32) = (960.0, 640.0);
/// The smallest window the sidebar's pages and footer links, and a page's rows, fit in.
pub const MIN_SIZE: (f32, f32) = (760.0, 480.0);

/// Room above the sidebar's rows for the traffic lights, which sit over a full-size content view.
const SIDEBAR_TOP: f32 = 38.0;
const SIDEBAR_WIDTH: f32 = 220.0;
const SIDEBAR_ROW_INSET: f32 = 6.0;
/// The page title's band, level with the traffic lights.
const TITLE_HEIGHT: f32 = 52.0;
/// Text on a page lines up this far in; cards reach `ROW_PX` past it.
const GUTTER: f32 = 32.0;

/// Settings' pages; Updates is reached through the sidebar footer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    General,
    Search,
    Keywords,
    Snippets,
    Snapshots,
    Appearance,
    Extensions,
    Permissions,
    Sync,
    Updates,
}

impl Page {
    fn title(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Search => "Search",
            Page::Keywords => "Keywords",
            Page::Snippets => "Snippets",
            Page::Snapshots => "Snapshots",
            Page::Appearance => "Appearance",
            Page::Extensions => "Extensions",
            Page::Permissions => "Permissions",
            Page::Sync => "Sync",
            Page::Updates => "Updates",
        }
    }

    /// The sidebar row's key, for scripts to press.
    fn key(self) -> &'static str {
        match self {
            Page::General => "page-general",
            Page::Search => "page-search",
            Page::Keywords => "page-keywords",
            Page::Snippets => "page-snippets",
            Page::Snapshots => "page-snapshots",
            Page::Appearance => "page-appearance",
            Page::Extensions => "page-extensions",
            Page::Permissions => "page-permissions",
            Page::Sync => "page-sync",
            Page::Updates => "page-updates",
        }
    }

    fn icon(self) -> &'static Icon {
        match self {
            Page::General => &icons::SLIDERS,
            Page::Search => &icons::SEARCH,
            Page::Keywords => &icons::KEYBOARD,
            Page::Snippets => &icons::QUOTE,
            Page::Snapshots => &icons::SNAPSHOT,
            Page::Appearance => &icons::SPARKLES,
            Page::Extensions => &icons::PUZZLE,
            Page::Permissions => &icons::SHIELD,
            Page::Sync => &icons::CLOUD,
            Page::Updates => &icons::REFRESH,
        }
    }
}

pub struct Prefs {
    config: Config,
    current: Rc<RefCell<Config>>,
    page: Page,
    /// The page ⌘, returns to: the last one shown other than Snapshots, which has its own way in.
    settings_page: Page,
    recorders: [ShortcutState; 2],
    navigation_recorders: [ShortcutState; 2],
    shortcut_problems: [Option<String>; 2],
    binding: Arc<Binding>,
    panel: AppHandle<Ui>,
    updater: Updater,
    locked_keys: crate::settings::LockedKeys,
    lock_note: &'static str,
    settings: Arc<crate::settings::Service>,
    config_problem: Option<String>,
    login: Option<LoginPreference>,
    /// Each visible permission's status in row order, read while the page shows.
    permissions: Vec<PermissionStatus>,
    calendar_access: CalendarAccess,
    /// Whether General shows its Advanced cards, kept while Dupin runs.
    advanced: bool,
    /// The first-run welcome's step while it covers the pages.
    welcome: Option<welcome::Step>,
    pub(crate) hotkey_problem: Option<String>,
    pub(crate) scripts_notice: Option<String>,
    preview_dark: Option<bool>,
    active_radius: u16,
    /// Loaded extensions, and why others were skipped.
    extensions: (Vec<crate::extensions::packages::Listed>, Vec<String>),
    /// The broker bridge the panel's extensions go through, once the panel has one.
    access: Option<dupin_extension_bridge::Bridge>,
    /// Why the last Allow, Revoke or switch on Extensions didn't apply.
    extension_problem: Option<String>,
    keywords: keywords::Keywords,
    snippets: snippets::Snippets,
    /// Settings itself, which a change written on a worker thread reports back to.
    me: AppHandle<Prefs>,
    sync: sync::SyncPane,
    /// The display the panel opens on, which the row count's recommendation is for.
    panel_screen: Option<crate::ui::PanelScreen>,
    pub(crate) snapshots: crate::snapshots::Snapshots,
}

impl Prefs {
    pub fn new(
        (current, locked_keys): (Rc<RefCell<Config>>, crate::settings::LockedKeys),
        binding: Arc<Binding>,
        (panel, me, running): (
            AppHandle<Ui>,
            AppHandle<Prefs>,
            Arc<std::sync::atomic::AtomicUsize>,
        ),
        updater: Updater,
        settings: Arc<crate::settings::Service>,
        welcome: bool,
        calendar_access: CalendarAccess,
    ) -> Prefs {
        let config = current.borrow().clone();
        let recorders = LauncherShortcut::ALL.map(|kind| {
            let mut recorder =
                ShortcutState::new(crate::hotkey::parse(kind.accelerator(&config)).ok().flatten());
            if binding.launch_problem(kind, recorder.shortcut()).is_some() {
                recorder.mark_unregistered();
            }
            recorder
        });
        let navigation_recorders = [
            ShortcutState::new(parse_navigation_shortcut(&config.navigation_up)),
            ShortcutState::new(parse_navigation_shortcut(&config.navigation_down)),
        ];
        let active_radius = config.theme.corner_radius;
        Prefs {
            preview_dark: None,
            active_radius,
            keywords: keywords::Keywords::new(&config),
            snippets: snippets::Snippets::new(&config),
            snapshots: crate::snapshots::Snapshots::new(panel.clone(), me.clone(), running),
            me,
            sync: sync::SyncPane::default(),
            panel_screen: crate::ui::PanelScreen::current(config.display),
            config,
            current,
            page: Page::General,
            settings_page: Page::General,
            recorders,
            navigation_recorders,
            shortcut_problems: [None, None],
            binding,
            panel,
            updater,
            locked_keys,
            lock_note: config_lock_note(&settings),
            settings,
            config_problem: None,
            extensions: Default::default(),
            access: None,
            extension_problem: None,
            hotkey_problem: None,
            scripts_notice: None,
            login: LoginPreference::current(),
            permissions: Vec::new(),
            calendar_access,
            advanced: false,
            welcome: welcome.then_some(welcome::Step::Hotkey),
        }
    }

    pub fn refresh_permissions(&mut self) {
        if self.welcome.is_none() && matches!(self.page, Page::Permissions | Page::Extensions) {
            self.permissions = permissions::read(&self.calendar_access);
            if let Some(bridge) = &self.access {
                crate::extensions::access::refresh_os_access(bridge, &self.calendar_access);
            }
        }
    }

    fn show(&mut self, page: Page) {
        self.welcome = None;
        self.page = page;
        if page != Page::Snapshots {
            self.settings_page = page;
        }
        self.refresh_permissions();
        if page == Page::Sync {
            self.sync_ask(crate::control::SyncAction::Status {});
        }
    }

    /// The panel opened on another display.
    pub fn reflect_panel_screen(&mut self, screen: Option<crate::ui::PanelScreen>) {
        self.panel_screen = screen;
    }

    /// Opens Settings as ⌘, does: on the page last shown, unless that was Snapshots, which
    /// ⇧⌘O opens instead, so Settings comes back to the settings page left before it.
    pub fn open_settings(&mut self) {
        if self.welcome.is_none() && self.page == Page::Snapshots {
            self.show(self.settings_page);
        }
        self.me.open();
    }

    /// Shows the Snapshots page, opening Settings if it isn't.
    pub fn open_snapshots(&mut self) {
        self.show(Page::Snapshots);
        self.me.open();
    }

    /// Closes Settings if it shows the Snapshots page, as `dupin snapshots close` asks.
    pub fn close_snapshots(&mut self, call: crate::control::Call) {
        self.snapshots.cancel_delete();
        if self.welcome.is_none() && self.page == Page::Snapshots {
            self.me.close();
        }
        call.finish(Ok(serde_json::Value::Null));
    }

    /// Keys no focused element took, which the Snapshots page uses while it shows.
    pub fn key(&mut self, ev: &rune::KeyEvent, tx: &mut Tx) -> bool {
        self.welcome.is_none() && self.page == Page::Snapshots && self.snapshots.key(ev, tx)
    }

    pub fn reflect_extensions(
        &mut self,
        listed: Vec<crate::extensions::packages::Listed>,
        problems: Vec<String>,
    ) {
        self.extensions = (listed, problems);
    }

    pub fn reflect_config(
        &mut self,
        revision: u64,
        config: Config,
        locked_keys: crate::settings::LockedKeys,
        problem: Option<String>,
    ) {
        if revision != self.settings.revision() {
            return;
        }
        self.adopt(config);
        self.locked_keys = locked_keys;
        self.lock_note = config_lock_note(&self.settings);
        self.config_problem = problem;
    }

    fn locked(&self, key: &str) -> bool {
        self.locked_keys.contains(key)
    }

    fn lock_note(&self, locked: bool) -> Option<&'static str> {
        locked.then_some(self.lock_note)
    }

    fn adopt(&mut self, config: Config) {
        for kind in LauncherShortcut::ALL {
            let i = kind as usize;
            let shortcut = crate::hotkey::parse(kind.accelerator(&config)).ok().flatten();
            if self.recorders[i].shortcut() != shortcut.as_ref() {
                self.recorders[i] = ShortcutState::new(shortcut);
                self.shortcut_problems[i] = None;
            }
        }
        for (recorder, value) in self
            .navigation_recorders
            .iter_mut()
            .zip([&config.navigation_up, &config.navigation_down])
        {
            let shortcut = parse_navigation_shortcut(value);
            if recorder.shortcut() != shortcut.as_ref() {
                *recorder = ShortcutState::new(shortcut);
            }
        }
        self.reflect_launch_hotkey();
        if config != self.config {
            self.keywords.reflect(&config, Some(&self.config));
            self.snippets.reflect(&config);
        }
        self.config = config;
    }

    /// The panel registers hotkeys after Settings constructs its recorders.
    pub fn reflect_launch_hotkey(&mut self) {
        for kind in LauncherShortcut::ALL {
            let recorder = &mut self.recorders[kind as usize];
            if self.binding.launch_problem(kind, recorder.shortcut()).is_some() {
                recorder.mark_unregistered();
            }
        }
    }

    pub fn refresh_hotkey(&mut self) {
        for kind in LauncherShortcut::ALL {
            let restored = self.binding.refresh_launch_conflict(kind);
            let recorder = &mut self.recorders[kind as usize];
            if let Some(refused) = recorder.refused()
                && Some(refused) == recorder.shortcut()
                && recorder.error() == Some(&ShortcutError::Conflict)
            {
                self.shortcut_problems[kind as usize] =
                    self.binding.recorder_conflict(kind, refused, &self.config);
                if restored {
                    recorder.cancel();
                }
            }
        }
    }

    fn set_problem(&mut self, problem: Option<String>) {
        if self.config_problem != problem
            && let Some(problem) = &problem
        {
            crate::log::warn(problem);
        }
        self.config_problem = problem.clone();
        self.panel.update(move |ui, _| ui.config_problem = problem);
    }

    pub fn closed(&mut self, _: &mut Tx) {
        // Closing the window ends the welcome, so ⌘, next opens Settings itself.
        self.welcome = None;
        // Queue the decision so Rune publishes Settings' hidden state before it runs.
        self.panel.update(Ui::resume_restart);
    }

    fn change_hotkey(
        &mut self,
        kind: LauncherShortcut,
        next: Option<&Shortcut>,
    ) -> Result<(), ShortcutError> {
        let base = self.current.borrow().clone();
        let result =
            if next.is_some_and(|shortcut| crate::hotkey::binding_targets(&base).contains_key(shortcut)) {
                Err(ShortcutError::Conflict)
            } else {
                self.binding.replace(kind, next)
            };
        self.shortcut_problems[kind as usize] = next
            .filter(|_| result == Err(ShortcutError::Conflict))
            .and_then(|next| self.binding.recorder_conflict(kind, next, &base));
        result?;
        self.config = base.clone();
        kind.set(&mut self.config, next);
        self.save(&base);
        Ok(())
    }

    fn change_navigation_key(&mut self, index: usize, next: Option<&Shortcut>) -> Result<(), ShortcutError> {
        let other = 1 - index;
        if next.is_some_and(|shortcut| self.navigation_recorders[other].shortcut() == Some(shortcut)) {
            return Err(ShortcutError::Conflict);
        }
        let base = self.current.borrow().clone();
        self.config = base.clone();
        let value = next.map(ToString::to_string).unwrap_or_default();
        match index {
            0 => self.config.navigation_up = value,
            1 => self.config.navigation_down = value,
            _ => return Err(ShortcutError::Backend("invalid navigation key".into())),
        }
        self.save(&base);
        Ok(())
    }

    fn set(&mut self, change: impl FnOnce(&mut Config)) {
        let base = self.current.borrow().clone();
        self.adopt(base.clone());
        change(&mut self.config);
        if let Err(error) = crate::settings::check_locked(&base, &self.config, &self.locked_keys) {
            self.adopt(base);
            self.set_problem(Some(error));
            return;
        }
        self.save(&base);
    }

    /// Persist the settings and hand them to the panel.
    fn save(&mut self, base: &Config) {
        self.refresh_keywords(base);
        *self.current.borrow_mut() = self.config.clone();
        // The panel must see this edit before the worker can echo its saved file back.
        self.publish();
        if let Err(error) = self.settings.save(base, &self.config) {
            self.set_problem(Some(error));
        }
        permissions::settings_changed(self, base);
    }

    fn publish(&self) {
        let config = self.config.clone();
        self.panel.update(move |ui, tx| ui.apply_config(&config, tx));
    }
}

fn config_lock_note(settings: &crate::settings::Service) -> &'static str {
    if settings
        .base_path()
        .and_then(|path| path.canonicalize().ok())
        .is_some_and(|path| path.starts_with("/nix/store"))
    {
        "Managed by Nix"
    } else {
        "Set in config.toml"
    }
}

fn parse_navigation_shortcut(value: &str) -> Option<Shortcut> {
    if value.trim().is_empty() {
        None
    } else {
        value.parse().ok()
    }
}

pub fn view(s: &Prefs, cx: &mut Cx<Prefs>) -> El {
    let k = rune_theme_opencode::kit(cx.system_dark());
    let hair = 1.0 / cx.env().scale;
    let rows = match (s.welcome, s.page) {
        (Some(step), _) => welcome::page(s, k, step),
        (None, Page::General) => general::page(s, k),
        (None, Page::Search) => search::page(s, k),
        (None, Page::Keywords) => keywords::page(s, k),
        (None, Page::Snippets) => snippets::page(s, k),
        (None, Page::Snapshots) => Vec::new(),
        (None, Page::Appearance) => appearance::page(s, k, cx.system_dark()),
        (None, Page::Extensions) => extensions::problem(s, k)
            .into_iter()
            .chain(extensions::page(s, k))
            .collect(),
        (None, Page::Permissions) => permissions::page(s, k),
        (None, Page::Sync) => sync::page(s, k),
        (None, Page::Updates) => updates::page(s, k),
    };
    let title = s.welcome.map_or(s.page.title(), welcome::Step::title);
    // The Snapshots page scrolls its list and its detail apart, so it fills the body instead.
    let snapshots = (s.welcome.is_none() && s.page == Page::Snapshots).then(|| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        crate::snapshots::page(&s.snapshots, k, now)
    });
    let body = col()
        .key("page-body")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(GUTTER - ROW_PX)
        .gap(12.0)
        .children(
            [
                ("config-problem", s.config_problem.as_deref()),
                ("hotkey-problem", s.hotkey_problem.as_deref()),
            ]
            .into_iter()
            .filter_map(|(key, problem)| Some((key, problem?)))
            .map(|(key, problem)| {
                text(problem)
                    .key(key)
                    .w_full()
                    .font(DESCRIPTION)
                    .color(k.c.text_base)
                    .wrap()
            }),
        )
        .children(
            crate::stored::problems()
                .into_iter()
                .enumerate()
                .map(|(index, problem)| {
                    text(problem)
                        .key(format!("storage-problem-{index}"))
                        .w_full()
                        .font(DESCRIPTION)
                        .color(k.c.text_base)
                        .wrap()
                }),
        )
        .children(rows);
    let accessory = snapshots.is_some().then(|| s.snapshots.count(k)).flatten();
    let body = match snapshots {
        Some(page) => body.child(page.mb(GUTTER - ROW_PX)),
        None => body.scroll(ScrollSpec { stick_bottom: false }),
    };
    let panel = col()
        .key("panel")
        .role("main")
        .grow(1.0)
        .min_w(0.0)
        .mt(8.0)
        .mr(8.0)
        .mb(8.0)
        .radius(k.radius.lg)
        .fill(k.c.bg_base)
        .border(hair, k.c.border_weak)
        .padding(hair)
        .child(
            row()
                .key("page-title")
                .w_full()
                .h(TITLE_HEIGHT)
                .px(GUTTER)
                .center_y()
                .window_drag()
                .child(
                    text(title)
                        .role("heading")
                        .font(k.ty.body_medium)
                        .color(k.c.text_base),
                )
                .children(accessory.map(|count| row().grow(1.0).child(el().grow(1.0)).child(count))),
        )
        .child(body)
        .children(s.welcome.map(|step| welcome::footer(k, step)));
    row()
        .key("prefs")
        .w_full()
        .h_full()
        .fill(k.c.bg_deep)
        .child(sidebar(s, k))
        .child(panel)
}

fn sidebar(s: &Prefs, k: &Kit) -> El {
    let c = &k.c;
    let mut pages = col()
        .key("pages")
        .role("list")
        .focus_group()
        .focus_follows()
        .gap(4.0);
    let permissions = permissions::available().then_some(Page::Permissions);
    for page in [
        Page::General,
        Page::Search,
        Page::Keywords,
        Page::Snippets,
        Page::Snapshots,
        Page::Appearance,
        Page::Extensions,
    ]
    .into_iter()
    .chain(permissions)
    .chain([Page::Sync])
    {
        let on = s.welcome.is_none() && s.page == page;
        let mut item = row()
            .key(page.key())
            .role("listitem")
            .label(page.title())
            .focus_selected(on)
            .focusable(true)
            .focus_ring(2.0, 0.0, c.border_focus)
            .h(28.0)
            .px(SIDEBAR_ROW_INSET)
            .gap(8.0)
            .center_y()
            .radius(k.radius.sm)
            .hover(c.hover)
            .press(c.pressed)
            .on_press(move |s: &mut Prefs| s.show(page))
            .child(icon(page.icon(), 16.0).color(if on { c.icon_base } else { c.icon_muted }))
            .child(
                text(page.title())
                    .font(k.ty.ui)
                    .color(if on { c.text_base } else { c.text_muted })
                    .truncate(),
            );
        if on {
            item = item.fill(c.layer_02);
        }
        pages = pages.child(item);
    }
    col()
        .key("sidebar")
        .role("navigation")
        .w(SIDEBAR_WIDTH)
        .h_full()
        .pt(SIDEBAR_TOP)
        .pb(12.0)
        .pl(10.0)
        .pr(6.0)
        .window_drag()
        .child(pages)
        .child(el().grow(1.0))
        .child(
            footer_link(k, "show-welcome", "Show Welcome")
                .on_press(|s: &mut Prefs| s.welcome = Some(welcome::Step::Hotkey)),
        )
        .children(crate::notices::path().map(|path| {
            footer_link(k, "open-acknowledgements", "Acknowledgements").on_press(move |_: &mut Prefs| {
                std::thread::spawn(move || dupin_mac::workspace::open(path));
            })
        }))
        .children(s.updater.available().then(|| updates::footer(s, k)))
}

/// Open config.toml in its default editor, when there is one.
fn open_config_file() {
    if let Some(path) = crate::settings::path()
        && path.exists()
    {
        std::thread::spawn(move || dupin_mac::workspace::open(&path));
    }
}

/// Open the folder holding config.toml, `extensions/` and `shaders/` in Finder, making it
/// first so a fresh install has somewhere to put them.
fn open_config_folder() {
    let Some(dir) = crate::settings::path().and_then(|path| path.parent().map(Path::to_path_buf)) else {
        return;
    };
    std::thread::spawn(move || {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            crate::log::warn(&format!("couldn't create {}: {e}", dir.display()));
            return;
        }
        _ = dupin_mac::workspace::open(&dir);
    });
}

/// A quiet link at the foot of the sidebar.
fn footer_link(k: &Kit, key: &str, label: &str) -> El {
    let c = &k.c;
    row()
        .key(key)
        .role("button")
        .label(label)
        .focusable(true)
        .focus_ring(2.0, 0.0, c.border_focus)
        .h(20.0)
        .px(SIDEBAR_ROW_INSET)
        .self_start()
        .center_y()
        .radius(k.radius.sm)
        .hover(c.hover)
        .press(c.pressed)
        .child(
            text(label.to_owned())
                .font(k.ty.small)
                .color(c.text_muted)
                .truncate(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn state(current: Rc<RefCell<Config>>, settings: Arc<crate::settings::Service>, welcome: bool) -> Prefs {
        let panel = rune::App::new(rune::AppSpec::new(
            "test panel",
            (1.0, 1.0),
            || unreachable!(),
            crate::view::view,
        ))
        .handle();
        let prefs = rune::App::new(rune::AppSpec::new(
            "test prefs",
            (1.0, 1.0),
            || unreachable!(),
            view,
        ))
        .handle();
        let binding = Arc::new(Binding::new(panel.clone()));
        let updater = Updater::start(false, panel.clone(), prefs.clone());
        Prefs::new(
            (current, Default::default()),
            binding,
            (panel, prefs, Default::default()),
            updater,
            settings,
            welcome,
            CalendarAccess::new(|| {}),
        )
    }

    #[test]
    fn locked_general_controls_are_disabled() {
        use rune::accesskit::Role;

        let current = Rc::new(RefCell::new(Config::default()));
        let settings = Arc::new(crate::settings::Service::new(Default::default(), |_| {}));
        let state = state(current, settings, false);
        let mut runtime = rune::Runtime::new(state, view, SIZE, rune::Env::default());
        let controls = [
            ("hotkey", "Show Dupin", Role::Button),
            ("selection_hotkey", "Search selected text", Role::Button),
            ("display", "Display", Role::ComboBox),
            ("system_automation", "System Command Automation", Role::Switch),
        ];
        let advanced = |runtime: &mut rune::Runtime<Prefs>| {
            runtime.frame(0.0);
            let tree = runtime.accessibility_tree();
            let shown = tree
                .nodes
                .iter()
                .any(|(_, node)| node.label() == Some("System Command Automation"));
            let (_, disclosure) = tree
                .nodes
                .iter()
                .find(|(_, node)| node.role() == Role::Button && node.label() == Some("Advanced"))
                .expect("the Advanced disclosure");
            assert_eq!(disclosure.is_expanded(), Some(shown));
            shown
        };
        assert!(!advanced(&mut runtime), "Advanced starts collapsed");
        let (x, y) = runtime
            .find("advanced")
            .expect("the Advanced disclosure is drawn");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        assert!(advanced(&mut runtime));

        for locked in [false, true] {
            runtime.update(|s, _| {
                s.locked_keys = controls
                    .iter()
                    .filter(|_| locked)
                    .map(|(key, _, _)| (*key).to_owned())
                    .collect();
            });
            runtime.frame(0.0);
            let tree = runtime.accessibility_tree();
            for (_, label, role) in controls {
                let control = tree
                    .nodes
                    .iter()
                    .find(|(_, node)| node.role() == role && node.label() == Some(label));
                let (_, control) = control.unwrap_or_else(|| panic!("missing control {label}"));
                assert_eq!(control.is_disabled(), locked, "{label}");
            }
        }
    }

    #[test]
    fn the_keywords_page_lists_commands_and_edits_aliases_unless_locked() {
        use rune::accesskit::Role;

        let config = Config::parse("[aliases]\nlookup = \"dictionary\"\nd = \"calendar\"").unwrap();
        let current = Rc::new(RefCell::new(config));
        let settings = Arc::new(crate::settings::Service::new(Default::default(), |_| {}));
        let mut state = state(current, settings, false);
        state.show(Page::Keywords);
        let mut runtime = rune::Runtime::new(state, view, SIZE, rune::Env::default());
        for locked in [false, true] {
            runtime.update(|s, _| {
                s.locked_keys = locked.then(|| "aliases".to_owned()).into_iter().collect();
            });
            runtime.frame(0.0);
            let tree = runtime.accessibility_tree();
            let has_label = |label: &str| tree.nodes.iter().any(|(_, node)| node.label() == Some(label));
            for label in ["Dictionary", "File Contents", "Search Google", "lookup", "d"] {
                assert!(has_label(label), "missing {label}");
            }
            let removes: Vec<_> = tree
                .nodes
                .iter()
                .filter(|(_, node)| node.role() == Role::Button && node.label() == Some("Remove"))
                .map(|(_, node)| node.is_disabled())
                .collect();
            assert_eq!(removes, [locked, locked]);
            assert_eq!(has_label("Add Alias"), !locked);
        }
        runtime.update(|s, _| {
            assert_eq!(
                s.keywords_problem("d"),
                Some("is already a keyword of Dictionary")
            );
            s.locked_keys = Default::default();
            s.set_alias_draft("ff", "notacommand!");
            s.add_alias();
            assert!(
                s.keywords
                    .problem
                    .as_deref()
                    .unwrap()
                    .contains("a command is named")
            );
            s.set_alias_draft("c", "calendar");
            s.add_alias();
            assert_eq!(s.keywords.problem, None);
            assert_eq!(
                s.config.aliases.0.get("c"),
                Some(&dupin_core::config::AliasTarget::Command("calendar".into()))
            );
            s.remove_alias("d");
            assert!(!s.config.aliases.0.contains_key("d"));
        });
    }

    #[test]
    fn a_shortcut_recorded_for_a_command_is_written_as_a_binding_unless_locked() {
        use dupin_core::config::HotkeyTarget;
        use rune::accesskit::Role;

        let config = Config::parse(
            "[bindings.\"ctrl+alt+c\"]\ntarget = \"app\"\nbundle_id = \"com.apple.calculator\"",
        )
        .unwrap();
        let current = Rc::new(RefCell::new(config));
        let settings = Arc::new(crate::settings::Service::new(Default::default(), |_| {}));
        let mut state = state(current, settings, false);
        state.show(Page::Keywords);
        let mut runtime = rune::Runtime::new(state, view, SIZE, rune::Env::default());
        let dictionary = |s: &Prefs| {
            s.config
                .bindings
                .iter()
                .filter(|(_, target)| matches!(target, HotkeyTarget::Dupin { .. }))
                .map(|(chord, target)| (chord.clone(), target.clone()))
                .collect::<Vec<_>>()
        };
        runtime.update(|s, _| {
            let chord: Shortcut = "ctrl+alt+cmd+d".parse().unwrap();
            assert_eq!(s.change_command_shortcut("dictionary", Some(&chord)), Ok(()));
            assert_eq!(
                dictionary(s),
                [(
                    chord.to_string(),
                    HotkeyTarget::Dupin {
                        command: dupin_core::config::AliasTarget::Command("dictionary".into()).into(),
                        query: None,
                    }
                )]
            );
            for (taken, problem) in [
                ("cmd+space", "⌘Space is assigned to the panel shortcut."),
                (
                    "ctrl+alt+c",
                    "⌃⌥C is assigned to the binding for com.apple.calculator.",
                ),
            ] {
                let taken: Shortcut = taken.parse().unwrap();
                assert_eq!(
                    s.change_command_shortcut("dictionary", Some(&taken)),
                    Err(ShortcutError::Conflict)
                );
                assert_eq!(s.keywords.shortcut_problems["dictionary"], problem);
            }
            // Recording another chord replaces the first.
            let other: Shortcut = "ctrl+alt+cmd+e".parse().unwrap();
            assert_eq!(s.change_command_shortcut("dictionary", Some(&other)), Ok(()));
            assert_eq!(dictionary(s).len(), 1);
            assert_eq!(dictionary(s)[0].0, other.to_string());
            assert!(s.keywords.shortcut_problems.is_empty());
        });
        for locked in [false, true] {
            runtime.update(|s, _| {
                s.locked_keys = locked.then(|| "bindings".to_owned()).into_iter().collect();
            });
            runtime.frame(0.0);
            let tree = runtime.accessibility_tree();
            let (_, recorder) = tree
                .nodes
                .iter()
                .find(|(_, node)| node.role() == Role::Button && node.label() == Some("Open Dictionary"))
                .expect("Dictionary's shortcut recorder");
            assert_eq!(recorder.is_disabled(), locked);
            let clear = tree
                .nodes
                .iter()
                .any(|(_, node)| node.role() == Role::Button && node.label() == Some("Clear"));
            assert_eq!(clear, !locked);
        }
        runtime.update(|s, _| {
            s.locked_keys = Default::default();
            assert_eq!(s.change_command_shortcut("dictionary", None), Ok(()));
            assert!(dictionary(s).is_empty());
            assert_eq!(s.config.bindings.len(), 1, "the app binding stays");
        });

        let press = |runtime: &mut rune::Runtime<Prefs>, key: &str| {
            runtime.frame(0.0);
            let (x, y) = runtime.find(key).unwrap_or_else(|| panic!("{key} is drawn"));
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.0);
        };
        press(&mut runtime, "shortcut-dictionary");
        assert!(runtime.state.keywords.shortcuts["dictionary"].is_recording());
        runtime.physical_input(rune::PhysicalInput::Key {
            key: rune::PhysicalKeyCode::KeyD,
            mods: rune::Mods {
                ctrl: true,
                alt: true,
                ..rune::Mods::default()
            },
            pressed: true,
            repeat: false,
            text: None,
            consumed_mods: Default::default(),
        });
        runtime.frame(0.0);
        assert!(!runtime.state.keywords.shortcuts["dictionary"].is_recording());
        assert_eq!(dictionary(&runtime.state)[0].0, "ctrl+alt+d");
        press(&mut runtime, "shortcut-dictionary-clear");
        assert!(dictionary(&runtime.state).is_empty());
        assert_eq!(runtime.state.keywords.shortcuts["dictionary"].shortcut(), None);
    }

    #[test]
    fn system_commands_web_searches_and_apps_record_shortcuts_too() {
        use dupin_core::config::{Address, HotkeyTarget};
        use rune::accesskit::Role;

        let config =
            Config::parse("[bindings.\"alt+cmd+s\"]\ntarget = \"dupin\"\ncommand = \"system/sleep\"")
                .unwrap();
        let current = Rc::new(RefCell::new(config));
        let settings = Arc::new(crate::settings::Service::new(Default::default(), |_| {}));
        let mut state = state(current, settings, false);
        state.show(Page::Keywords);
        let mut runtime = rune::Runtime::new(state, view, SIZE, rune::Env::default());
        runtime.frame(0.0);
        let tree = runtime.accessibility_tree();
        let recorder = |label: &str| {
            tree.nodes
                .iter()
                .any(|(_, node)| node.role() == Role::Button && node.label() == Some(label))
        };
        assert!(recorder("Run Sleep") && recorder("Run Lock Screen") && recorder("Run Restart Dupin"));
        assert!(recorder("Open Search Google"));
        runtime.update(|s, _| {
            let bound = |s: &Prefs, address: &str| {
                let address = Address::parse(address).unwrap();
                s.config
                    .bindings
                    .iter()
                    .find(|(_, target)| matches!(target, HotkeyTarget::Dupin { command, query: None } if *command == address))
                    .map(|(chord, _)| chord.clone())
            };
            assert_eq!(bound(s, "system/sleep").as_deref(), Some("alt+cmd+s"));
            let taken: Shortcut = "alt+cmd+s".parse().unwrap();
            assert_eq!(
                s.change_command_shortcut("system/lock-screen", Some(&taken)),
                Err(ShortcutError::Conflict)
            );
            assert_eq!(
                s.keywords.shortcut_problems["system/lock-screen"],
                "⌥⌘S is assigned to the binding for system/sleep."
            );
            for (address, chord) in [
                ("system/lock-screen", "ctrl+alt+cmd+l"),
                ("web:g", "ctrl+alt+cmd+g"),
                ("app:org.mozilla.firefox", "ctrl+alt+cmd+f"),
            ] {
                let chord: Shortcut = chord.parse().unwrap();
                assert_eq!(s.change_command_shortcut(address, Some(&chord)), Ok(()));
                assert_eq!(bound(s, address), Some(chord.to_string()), "{address}");
            }
            assert_eq!(s.change_command_shortcut("system/sleep", None), Ok(()));
            assert_eq!(bound(s, "system/sleep"), None);
        });
    }

    #[test]
    fn a_store_that_failed_closed_is_named_on_every_page() {
        const STORE: &str = "A store under test";
        let shown = |runtime: &mut rune::Runtime<Prefs>| {
            runtime.frame(0.0);
            runtime.accessibility_tree().nodes.iter().any(|(_, node)| {
                node.value()
                    .is_some_and(|text| text.starts_with(STORE) && text.contains("storage key"))
            })
        };
        let current = Rc::new(RefCell::new(Config::default()));
        let settings = Arc::new(crate::settings::Service::new(Default::default(), |_| {}));
        crate::stored::report(STORE, Some("couldn't get the storage key: locked"));
        let mut runtime =
            rune::Runtime::new(state(current, settings, false), view, SIZE, rune::Env::default());
        for page in [Page::General, Page::Keywords, Page::Extensions] {
            runtime.update(move |s, _| s.page = page);
            assert!(shown(&mut runtime), "{}", page.title());
        }
        crate::stored::report(STORE, None);
        runtime.update(|_, _| {});
        assert!(!shown(&mut runtime));
    }

    #[test]
    fn the_snapshots_page_lists_snapshots_beside_the_selected_one_and_keeps_its_keys() {
        let current = Rc::new(RefCell::new(Config::default()));
        let settings = Arc::new(crate::settings::Service::new(Default::default(), |_| {}));
        let mut state = state(current, settings, false);
        let escape = rune::KeyEvent {
            key: rune::Key::Escape,
            mods: Default::default(),
        };
        // Elsewhere in Settings the page's keys are left to the window.
        assert!(!state.key(&escape, &mut Tx::default()));
        state.show(Page::Snapshots);
        let mut runtime = rune::Runtime::new(state, view, SIZE, rune::Env::default());
        runtime.frame(0.0);
        let has_label = |runtime: &rune::Runtime<Prefs>, label: &str| {
            runtime
                .accessibility_tree()
                .nodes
                .iter()
                .any(|(_, node)| node.label() == Some(label) || node.value() == Some(label))
        };
        assert!(has_label(&runtime, "No snapshots yet"));

        let snapshot = |id: &str, title: &str, saved_at| dupin_core::snapshots::Snapshot {
            version: 1,
            id: id.into(),
            saved_at,
            mode: "dictionary.lookup".into(),
            explicit: true,
            query: title.into(),
            title: title.into(),
            views: vec![dupin_core::View::Detail {
                title: title.into(),
                body: format!("The meaning of {title}"),
            }],
            state: None,
        };
        runtime.update(|s, tx| {
            s.snapshots.seed(
                vec![snapshot("new", "serendipity", 2), snapshot("old", "petrichor", 1)],
                tx,
            );
        });
        runtime.frame(0.0);
        for label in [
            "Search saved snapshots",
            "serendipity",
            "petrichor",
            "The meaning of serendipity",
        ] {
            assert!(has_label(&runtime, label), "missing {label}");
        }
        assert!(has_label(&runtime, "2 of 100 saved"));
        runtime.update(|s, tx| {
            assert!(s.key(
                &rune::KeyEvent {
                    key: rune::Key::Down,
                    mods: Default::default(),
                },
                tx
            ));
        });
        runtime.frame(0.0);
        assert!(has_label(&runtime, "The meaning of petrichor"));
        assert!(!has_label(&runtime, "The meaning of serendipity"));
    }

    #[test]
    fn settings_opened_after_snapshots_comes_back_to_the_settings_page_tab_reaches() {
        let current = Rc::new(RefCell::new(Config::default()));
        let settings = Arc::new(crate::settings::Service::new(Default::default(), |_| {}));
        let mut state = state(current, settings, false);
        state.show(Page::Keywords);
        state.show(Page::Snapshots);
        // ⌘, opens Settings itself; ⇧⌘O is the way to Snapshots.
        state.open_settings();
        assert!(state.page == Page::Keywords);
        state.show(Page::General);
        state.show(Page::Snapshots);
        state.open_settings();
        let mut runtime = rune::Runtime::new(state, view, SIZE, rune::Env::default());
        runtime.on_key = Some(Prefs::key);
        runtime.frame(0.0);
        runtime.key(rune::KeyEvent {
            key: rune::Key::Tab,
            mods: Default::default(),
        });
        runtime.frame(0.0);
        let focused: Vec<_> = runtime
            .elements()
            .into_iter()
            .filter(|element| element.focused)
            .filter_map(|element| element.key)
            .collect();
        assert_eq!(focused, ["page-general"]);
    }

    #[test]
    fn the_snippets_page_lists_config_toml_snippets_read_only_and_adds_stored_ones() {
        use rune::accesskit::Role;

        let config =
            Config::parse("[snippets.sig]\nname = \"Email signature\"\ntext = \"Best wishes\"").unwrap();
        let current = Rc::new(RefCell::new(config));
        let settings = Arc::new(crate::settings::Service::new(Default::default(), |_| {}));
        let mut state = state(current, settings, false);
        state.show(Page::Snippets);
        assert_eq!(
            state.snippet_rows(),
            [("sig".to_owned(), dupin_core::snippets::Source::Config)]
        );
        let mut runtime = rune::Runtime::new(state, view, SIZE, rune::Env::default());
        runtime.frame(0.0);
        let tree = runtime.accessibility_tree();
        let button = |label: &str| {
            tree.nodes
                .iter()
                .any(|(_, node)| node.role() == Role::Button && node.label() == Some(label))
        };
        assert!(button("Add Snippet"));
        assert!(!button("Edit"), "config.toml's snippet is never edited here");
        assert!(!button("Delete"));
        assert!(
            tree.nodes
                .iter()
                .any(|(_, node)| node.label() == Some("Email signature"))
        );
        runtime.update(|s, _| s.add_snippet());
        runtime.frame(0.0);
        let tree = runtime.accessibility_tree();
        for label in ["Name", "Keyword", "Text", "Save Snippet"] {
            assert!(
                tree.nodes.iter().any(|(_, node)| node.label() == Some(label)),
                "missing {label}"
            );
        }
    }

    #[test]
    fn clipboard_options_follow_the_history_switch_and_its_lock() {
        use rune::accesskit::Role;

        let current = Rc::new(RefCell::new(Config::default()));
        let settings = Arc::new(crate::settings::Service::new(Default::default(), |_| {}));
        let mut state = state(current, settings, false);
        state.show(Page::Search);
        let mut runtime = rune::Runtime::new(state, view, SIZE, rune::Env::default());
        let controls = [
            ("Clipboard history", Role::Switch),
            ("Keep copies for", Role::ComboBox),
            ("Clipboard images", Role::Switch),
            ("Clipboard files", Role::Switch),
        ];

        for (enabled, locked) in [(true, false), (false, false), (true, true)] {
            runtime.update(|s, _| {
                s.config.clipboard.enabled = enabled;
                s.locked_keys = locked.then(|| "clipboard".to_owned()).into_iter().collect();
            });
            runtime.frame(0.0);
            let tree = runtime.accessibility_tree();
            for (index, (label, role)) in controls.into_iter().enumerate() {
                let (_, control) = tree
                    .nodes
                    .iter()
                    .find(|(_, node)| node.role() == role && node.label() == Some(label))
                    .unwrap_or_else(|| panic!("missing control {label}"));
                let disabled = locked || (index > 0 && !enabled);
                assert_eq!(control.is_disabled(), disabled, "{label} {enabled} {locked}");
            }
        }
    }

    #[test]
    fn the_extensions_page_lists_each_extension_with_its_switch_and_problems() {
        use crate::extensions::packages::{Kind, Listed};
        use dupin_extension_bridge::{Bridge, Broker, Capabilities, PackageId, Revision, Services};
        use rune::accesskit::Role;

        let current = Rc::new(RefCell::new(Config::default()));
        let settings = Arc::new(crate::settings::Service::new(Default::default(), |_| {}));
        let mut state = state(current, settings, false);
        let bridge = Bridge::new(
            Broker::new(Default::default(), Default::default()),
            None,
            Services::none(),
        );
        let (dupin, words) = (
            PackageId::new("bundled:dupin").unwrap(),
            PackageId::new("words").unwrap(),
        );
        let revision = Revision::new("blake3:fixture").unwrap();
        bridge.set_bundled(&dupin, revision.clone(), true);
        bridge.install(&dupin, revision.clone(), Capabilities::default());
        bridge.install(&words, revision, Capabilities::default());
        bridge.set_enabled(&words, false).unwrap();
        let listed = |package: &PackageId, title: &str, kind| Listed {
            package: package.clone(),
            title: title.into(),
            keyword: Some(title.to_lowercase()),
            icon: None,
            kind,
            registered: true,
            main_search: false,
            built_in: false,
        };
        state.reflect_extensions(
            vec![
                listed(&dupin, "Dupin", Kind::Bundled),
                listed(&words, "Words", Kind::Modified),
            ],
            vec!["fixture: invalid manifest".into()],
        );
        state.reflect_extension_access(bridge);
        // Not `show`, which reads macOS access.
        state.page = Page::Extensions;
        let mut runtime = rune::Runtime::new(state, view, SIZE, rune::Env::default());
        runtime.frame(0.0);

        let keys: Vec<_> = runtime
            .elements()
            .iter()
            .filter_map(|element| element.key.clone())
            .collect();
        for key in ["extension-0", "extension-1", "extension-problems"] {
            assert!(keys.iter().any(|k| k == key), "{key}");
        }
        let tree = runtime.accessibility_tree();
        for (label, on) in [("Dupin", true), ("Words", false), ("Extensions folder", false)] {
            let (_, switch) = tree
                .nodes
                .iter()
                .find(|(_, node)| node.role() == Role::Switch && node.label() == Some(label))
                .unwrap_or_else(|| panic!("missing switch {label}"));
            assert_eq!(
                switch.toggled() == Some(rune::accesskit::Toggled::True),
                on,
                "{label}"
            );
        }
    }

    #[test]
    fn extensions_lists_each_declared_scope_with_its_standing_and_waiting_requests() {
        use dupin_extension_bridge::{
            Bridge, Broker, Capabilities, ConsentState, PackageId, Revision, Services, Trigger,
        };

        let current = Rc::new(RefCell::new(Config::default()));
        let settings = Arc::new(crate::settings::Service::new(Default::default(), |_| {}));
        let mut state = state(current, settings, false);
        let bridge = Bridge::new(
            Broker::new(Default::default(), Default::default()),
            None,
            Services::none(),
        );
        let id = PackageId::new("weather").unwrap();
        let revision = Revision::new("blake3:fixture").unwrap();
        bridge.install(
            &id,
            revision.clone(),
            Capabilities::parse(&serde_json::json!({
                "http": [{ "origin": "https://api.example.com", "methods": ["GET"] }],
                "dictionary": ["lookup"],
            }))
            .unwrap(),
        );
        bridge.set_enabled(&id, true).unwrap();
        let http = bridge.scopes(&id)[0].scope.clone();
        bridge.allow(&id, &revision, &http, None).unwrap();
        // A request that needs consent leaves one waiting, as a guest's would.
        let call = bridge.call(&id, Trigger::Query);
        let asked = dupin_extension_bridge::HostRequest { run: 1, call: 1 };
        let reply = call.services.call(
            asked,
            "dictionary.lookup",
            br#"{"word":"dupin"}"#,
            std::time::Instant::now() + Duration::from_secs(1),
            &Default::default(),
        );
        call.services.ended(asked.run, &[asked.call]);
        assert!(String::from_utf8(reply).unwrap().contains("needs-consent"));
        state.reflect_extensions(
            vec![crate::extensions::packages::Listed {
                package: id.clone(),
                title: "Weather".into(),
                keyword: Some("wx".into()),
                icon: None,
                kind: crate::extensions::packages::Kind::User,
                registered: true,
                main_search: false,
                built_in: false,
            }],
            Vec::new(),
        );
        state.reflect_extension_access(bridge.clone());
        // Not `show`, which reads macOS access, and the pasteboard is out of reach in the Nix
        // build sandbox.
        state.page = Page::Extensions;
        let mut runtime = rune::Runtime::new(state, view, SIZE, rune::Env::default());
        runtime.frame(0.0);
        let keys: Vec<_> = runtime
            .elements()
            .iter()
            .filter_map(|element| element.key.clone())
            .collect();
        for key in [
            "waiting",
            "pending-0-allow",
            "pending-0-deny",
            "extension-0",
            "extension-0-enabled",
            "extension-0-scope-0-revoke",
            "extension-0-scope-1-allow",
        ] {
            assert!(keys.iter().any(|found| found == key), "missing {key} in {keys:?}");
        }
        let labels: Vec<_> = runtime
            .accessibility_tree()
            .nodes
            .iter()
            .filter_map(|(_, node)| node.label().map(str::to_owned))
            .collect();
        assert!(
            labels
                .iter()
                .any(|label| label == "Weather wants to look words up in the system dictionaries"),
            "{labels:?}"
        );

        // Revoking from Settings takes the grant away at once.
        let (x, y) = runtime.find("extension-0-scope-0-revoke").unwrap();
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.0);
        assert_eq!(bridge.scopes(&id)[0].state, ConsentState::Revoked);
    }

    #[test]
    fn both_updates_footer_buttons_leave_the_welcome() {
        let current = Rc::new(RefCell::new(Config::default()));
        let settings = Arc::new(crate::settings::Service::new(Default::default(), |_| {}));
        let state = state(current, settings, true);
        let mut runtime = rune::Runtime::new(
            state,
            |s, cx| updates::footer(s, rune_theme_opencode::kit(cx.system_dark())),
            (SIDEBAR_WIDTH, 100.0),
            rune::Env::default(),
        );

        for key in ["current-version", Page::Updates.key()] {
            runtime.update(|s, _| {
                s.page = Page::General;
                s.welcome = Some(welcome::Step::Hotkey);
            });
            runtime.frame(0.0);
            let (x, y) = runtime.find(key).unwrap();
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.0);
            runtime.update(|s, _| {
                assert!(s.page == Page::Updates, "{key} should open Updates");
                assert_eq!(s.welcome, None, "{key} should dismiss Welcome");
            });
        }
    }

    #[test]
    fn editing_stale_preferences_preserves_the_latest_external_values() {
        let path = std::env::temp_dir().join(format!(
            "dupin-prefs-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let (sent, received) = mpsc::channel();
        let settings = Arc::new(crate::settings::Service::told(
            crate::settings::Paths {
                base: Some(path.clone()),
                overlay: None,
            },
            move |update| sent.send(update).unwrap(),
        ));
        let current = Rc::new(RefCell::new(Config::default()));
        let mut state = state(current.clone(), settings.clone(), false);
        let external = Config {
            hotkey: "ctrl+alt+k".into(),
            dictionary_keywords: vec!["lookup".into()],
            ..Config::parse(
                "[bindings.\"ctrl+alt+c\"]\ntarget = \"app\"\nbundle_id = \"com.apple.calculator\"",
            )
            .unwrap()
        };
        std::fs::write(&path, toml::to_string(&external).unwrap()).unwrap();
        *current.borrow_mut() = external.clone();
        state.set(|config| config.file_limit = 123);
        let expected = Config {
            file_limit: 123,
            ..external
        };
        assert_eq!(*current.borrow(), expected);
        assert_eq!(
            state.recorders[LauncherShortcut::Panel as usize].shortcut(),
            crate::hotkey::parse(&expected.hotkey).unwrap().as_ref()
        );
        let saved = received.recv().unwrap();
        assert_eq!(saved.result.as_ref().unwrap(), &expected);
        state.reflect_config(saved.revision, expected.clone(), Default::default(), None);
        assert_eq!(
            state.recorders[LauncherShortcut::Panel as usize].shortcut(),
            crate::hotkey::parse(&expected.hotkey).unwrap().as_ref()
        );
        assert_eq!(saved.revision, settings.revision());
        let persisted = std::fs::read_to_string(&path).unwrap();
        assert_eq!(toml::from_str::<Config>(&persisted).unwrap(), expected);
        drop(state);
        drop(settings);
        std::fs::remove_file(path).unwrap();
    }
}
