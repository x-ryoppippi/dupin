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
mod snippets;
mod updates;
pub mod welcome;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use dupin_core::Config;
use dupin_mac::privacy::{CalendarAccess, PermissionStatus};
use rune::{AppHandle, Cx, El, Icon, ScrollSpec, Shortcut, ShortcutError, Tx, col, el, icon, row, text};
use rune_kit::{Kit, ShortcutRecorderState};

use self::controls::{DESCRIPTION, ROW_PX};
use self::general::LoginPreference;
use crate::hotkey::{Binding, LauncherShortcut};
use crate::icons;
use crate::ui::Ui;
use crate::update::Updater;

pub const SIZE: (f32, f32) = (760.0, 480.0);

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
    Appearance,
    Extensions,
    Permissions,
    Updates,
}

impl Page {
    fn title(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Search => "Search",
            Page::Keywords => "Keywords",
            Page::Snippets => "Snippets",
            Page::Appearance => "Appearance",
            Page::Extensions => "Extensions",
            Page::Permissions => "Permissions",
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
            Page::Appearance => "page-appearance",
            Page::Extensions => "page-extensions",
            Page::Permissions => "page-permissions",
            Page::Updates => "page-updates",
        }
    }

    fn icon(self) -> &'static Icon {
        match self {
            Page::General => &icons::SLIDERS,
            Page::Search => &icons::SEARCH,
            Page::Keywords => &icons::KEYBOARD,
            Page::Snippets => &icons::QUOTE,
            Page::Appearance => &icons::SPARKLES,
            Page::Extensions => &icons::PUZZLE,
            Page::Permissions => &icons::SHIELD,
            Page::Updates => &icons::REFRESH,
        }
    }
}

pub struct Prefs {
    config: Config,
    current: Rc<RefCell<Config>>,
    page: Page,
    recorders: [ShortcutRecorderState; 2],
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
}

impl Prefs {
    pub fn new(
        (current, locked_keys): (Rc<RefCell<Config>>, crate::settings::LockedKeys),
        binding: Arc<Binding>,
        (panel, me): (AppHandle<Ui>, AppHandle<Prefs>),
        updater: Updater,
        settings: Arc<crate::settings::Service>,
        welcome: bool,
        calendar_access: CalendarAccess,
    ) -> Prefs {
        let config = current.borrow().clone();
        let recorders = LauncherShortcut::ALL.map(|kind| {
            let mut recorder =
                ShortcutRecorderState::new(crate::hotkey::parse(kind.accelerator(&config)).ok().flatten());
            if binding.launch_problem(kind, recorder.shortcut()).is_some() {
                recorder.mark_unregistered();
            }
            recorder
        });
        let active_radius = config.theme.corner_radius;
        Prefs {
            preview_dark: None,
            active_radius,
            keywords: keywords::Keywords::new(&config),
            snippets: snippets::Snippets::new(&config),
            me,
            config,
            current,
            page: Page::General,
            recorders,
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
        self.refresh_permissions();
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
                self.recorders[i] = ShortcutRecorderState::new(shortcut);
                self.shortcut_problems[i] = None;
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

pub fn view(s: &Prefs, cx: &mut Cx<Prefs>) -> El {
    let k = rune_theme_opencode::kit(cx.system_dark());
    let hair = 1.0 / cx.env().scale;
    let rows = match (s.welcome, s.page) {
        (Some(step), _) => welcome::page(s, k, hair, step),
        (None, Page::General) => general::page(s, k, hair),
        (None, Page::Search) => search::page(s, k, hair),
        (None, Page::Keywords) => keywords::page(s, k, hair),
        (None, Page::Snippets) => snippets::page(s, k, hair),
        (None, Page::Appearance) => appearance::page(s, k, cx.system_dark()),
        (None, Page::Extensions) => extensions::problem(s, k)
            .into_iter()
            .chain(extensions::page(s, k, hair))
            .collect(),
        (None, Page::Permissions) => permissions::page(s, k, hair),
        (None, Page::Updates) => updates::page(s, k, hair),
    };
    let title = s.welcome.map_or(s.page.title(), welcome::Step::title);
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
                ),
        )
        .child(
            col()
                .key("page-body")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec { stick_bottom: false })
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
                .children(rows),
        )
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
        Page::Appearance,
        Page::Extensions,
    ]
    .into_iter()
    .chain(permissions)
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
    let config = footer_link(k, "open-config", "Open config.toml").on_press(|_: &mut Prefs| {
        if let Some(path) = crate::settings::path()
            && path.exists()
        {
            std::thread::spawn(move || dupin_mac::workspace::open(&path));
        }
    });
    let folder = footer_link(k, "open-config-folder", "Open Config Folder")
        .on_press(|_: &mut Prefs| open_config_folder());
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
        .child(
            footer_link(k, "open-snapshots", "Open Snapshots")
                .on_press(|s: &mut Prefs| s.panel.update(crate::ui::Ui::show_snapshots)),
        )
        .child(config)
        .child(folder)
        .children(s.updater.available().then(|| updates::footer(s, k)))
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
            (panel, prefs),
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
            keyword: title.to_lowercase(),
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
        let reply = call.services.call(
            "dictionary.lookup",
            br#"{"word":"dupin"}"#,
            std::time::Instant::now() + Duration::from_secs(1),
            &Default::default(),
        );
        assert!(String::from_utf8(reply).unwrap().contains("needs-consent"));
        state.reflect_extensions(
            vec![crate::extensions::packages::Listed {
                package: id.clone(),
                title: "Weather".into(),
                keyword: "wx".into(),
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
        let settings = Arc::new(crate::settings::Service::new(
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
        let saved = received.recv_timeout(Duration::from_secs(5)).unwrap();
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
