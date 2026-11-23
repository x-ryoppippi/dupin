//! The settings window (⌘, in the panel): a sidebar of pages, each a card of settings, drawn
//! in OpenCode's theme as Hex's settings are. A release build adds a page for its updates.
//! Changes apply to the panel at once and are written back to config.toml. A read-only
//! config.toml (one Nix manages) shows its values, locked.

mod appearance;
mod controls;
mod general;
mod permissions;
mod search;
mod updates;
pub mod welcome;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use dupin_core::Config;
use dupin_mac::privacy::{CalendarAccess, PermissionStatus};
use rune::{AppHandle, Cx, El, Icon, ScrollSpec, Shortcut, ShortcutError, Tx, col, el, icon, row, text};
use rune_kit::{Kit, ShortcutRecorderState};

use self::controls::{DESCRIPTION, ROW_PX, note};
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
    Appearance,
    Permissions,
    Updates,
}

impl Page {
    fn title(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Search => "Search",
            Page::Appearance => "Appearance",
            Page::Permissions => "Permissions",
            Page::Updates => "Updates",
        }
    }

    /// The sidebar row's key, for scripts to press.
    fn key(self) -> &'static str {
        match self {
            Page::General => "page-general",
            Page::Search => "page-search",
            Page::Appearance => "page-appearance",
            Page::Permissions => "page-permissions",
            Page::Updates => "page-updates",
        }
    }

    fn icon(self) -> &'static Icon {
        match self {
            Page::General => &icons::SLIDERS,
            Page::Search => &icons::SEARCH,
            Page::Appearance => &icons::SPARKLES,
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
    read_only: bool,
    settings: Arc<crate::settings::Service>,
    config_problem: Option<String>,
    login: Option<LoginPreference>,
    /// Each permission's status, in `permissions::ENTRIES` order, read while the page shows.
    permissions: Vec<PermissionStatus>,
    calendar_access: CalendarAccess,
    /// The first-run welcome's step while it covers the pages.
    welcome: Option<welcome::Step>,
    pub(crate) hotkey_problem: Option<String>,
    pub(crate) scripts_notice: Option<String>,
    preview_dark: Option<bool>,
    active_radius: u16,
    /// Loaded extensions as "Name (keyword)", and why others were skipped.
    extensions: (Vec<String>, Vec<String>),
}

impl Prefs {
    pub fn new(
        current: Rc<RefCell<Config>>,
        binding: Arc<Binding>,
        panel: AppHandle<Ui>,
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
            config,
            current,
            page: Page::General,
            recorders,
            shortcut_problems: [None, None],
            binding,
            panel,
            updater,
            read_only: crate::settings::read_only(),
            settings,
            config_problem: None,
            extensions: Default::default(),
            hotkey_problem: None,
            scripts_notice: None,
            login: LoginPreference::current(),
            permissions: Vec::new(),
            calendar_access,
            welcome: welcome.then_some(welcome::Step::Hotkey),
        }
    }

    pub fn refresh_permissions(&mut self) {
        if self.welcome.is_none() && self.page == Page::Permissions {
            self.permissions = permissions::read(&self.calendar_access);
        }
    }

    fn show(&mut self, page: Page) {
        self.welcome = None;
        self.page = page;
        self.refresh_permissions();
    }

    pub fn reflect_extensions(&mut self, loaded: Vec<String>, problems: Vec<String>) {
        self.extensions = (loaded, problems);
    }

    pub fn reflect_config(
        &mut self,
        revision: u64,
        config: Config,
        read_only: bool,
        problem: Option<String>,
    ) {
        if revision != self.settings.revision() {
            return;
        }
        self.adopt(config);
        self.read_only = read_only;
        self.config_problem = problem;
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
        self.save(&base);
    }

    /// Persist the settings and hand them to the panel.
    fn save(&mut self, base: &Config) {
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

pub fn view(s: &Prefs, cx: &mut Cx<Prefs>) -> El {
    let k = rune_theme_opencode::kit(cx.system_dark());
    let hair = 1.0 / cx.env().scale;
    let rows = match (s.welcome, s.page) {
        (Some(step), _) => welcome::page(s, k, hair, step),
        (None, Page::General) => general::page(s, k, hair),
        (None, Page::Search) => search::page(s, k, hair),
        (None, Page::Appearance) => appearance::page(s, k, cx.system_dark()),
        (None, Page::Permissions) => permissions::page(s, k, hair),
        (None, Page::Updates) => updates::page(s, k, hair),
    };
    let title = s.welcome.map_or(s.page.title(), welcome::Step::title);
    let shows_config = match s.welcome {
        Some(step) => step == welcome::Step::Hotkey,
        None => s.page != Page::Permissions,
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
                .children(rows)
                .children(
                    (s.read_only && shows_config)
                        .then(|| note(k, "config.toml is read-only, so change these where it's managed.")),
                ),
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
    for page in [Page::General, Page::Search, Page::Appearance]
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
        .children(s.updater.available().then(|| updates::footer(s, k)))
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
        let updater = Updater::start(false, panel.clone(), prefs);
        Prefs::new(
            current,
            binding,
            panel,
            updater,
            settings,
            welcome,
            CalendarAccess::new(|| {}),
        )
    }

    #[test]
    fn skipped_extension_problems_follow_the_extensions_switch() {
        let config = Config {
            snippet_paste: false,
            ..Config::default()
        };
        let current = Rc::new(RefCell::new(config));
        let settings = Arc::new(crate::settings::Service::new(None, |_| {}));
        let mut state = state(current, settings, false);
        state.reflect_extensions(Vec::new(), vec!["fixture: invalid manifest".into()]);
        state.show(Page::Search);
        let mut runtime = rune::Runtime::new(state, view, SIZE, rune::Env::default());

        for (extensions, window_management) in [(true, false), (false, true)] {
            runtime.update(|s, _| {
                s.config.extensions = extensions;
                s.config.window_management = window_management;
            });
            runtime.frame(0.0);
            let shows_problems = runtime
                .elements()
                .iter()
                .any(|element| element.key.as_deref() == Some("extension-problems"));
            assert_eq!(shows_problems, extensions);
        }
    }

    #[test]
    fn both_updates_footer_buttons_leave_the_welcome() {
        let current = Rc::new(RefCell::new(Config::default()));
        let settings = Arc::new(crate::settings::Service::new(None, |_| {}));
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
        let settings = Arc::new(crate::settings::Service::new(Some(path.clone()), move |update| {
            sent.send(update).unwrap();
        }));
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
        state.reflect_config(saved.revision, expected.clone(), false, None);
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
