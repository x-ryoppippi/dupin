//! The settings window (⌘, in the panel): a sidebar of pages, each a card of settings, drawn
//! in OpenCode's theme as Hex's settings are. A release build adds a page for its updates.
//! Changes apply to the panel at once and are written back to config.toml. A read-only
//! config.toml (one Nix manages) shows its values, locked.

mod permissions;
pub mod welcome;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use dupin_core::{Backdrop, Config, DisplaySelection, InputSource, Position};
use dupin_mac::login_item::{LoginItem, Status as LoginStatus};
use dupin_mac::privacy::Access;
use rune::{
    AppHandle, Cx, El, Icon, PopupItem, PopupMenu, ScrollSpec, Shortcut, ShortcutError, TextStyle, Tx, col,
    el, icon, row, text,
};
use rune_kit::{
    Button, Kit, SegmentedControl, Select, SelectAppearance, ShortcutRecorder, ShortcutRecorderState, State,
    Switch, shortcut_error_message,
};

use crate::hotkey::{Binding, LauncherShortcut};
use crate::icons;
use crate::shader::EXAMPLES;
use crate::ui::Ui;
use crate::update::{Status, Updater};

pub const SIZE: (f32, f32) = (760.0, 480.0);

/// Room above the sidebar's rows for the traffic lights, which sit over a full-size content view.
const SIDEBAR_TOP: f32 = 38.0;
const SIDEBAR_WIDTH: f32 = 220.0;
/// The page title's band, level with the traffic lights.
const TITLE_HEIGHT: f32 = 52.0;
/// Text on a page lines up this far in; cards reach `ROW_PX` past it.
const GUTTER: f32 = 32.0;
const ROW_PX: f32 = 16.0;
/// The widest a row's value grows before it truncates, leaving its name and description room.
const VALUE_MAX_W: f32 = 200.0;
const DESCRIPTION: TextStyle = TextStyle::new(12.0, 440.0).lh(16.0);

const FILE_LIMITS: [usize; 4] = [50, 100, 200, 500];

const POSITION: &str = "position";
const DISPLAY: &str = "display";
const INPUT_SOURCE: &str = "input-source";
const FILE_LIMIT: &str = "file-limit";
const BACKDROP: &str = "backdrop";
const FLOW: &str = "flow";
const OFF: &str = "off";
const EXAMPLE: &str = "example:";
const FILE: &str = "file:";

/// Where a row's menu opens: its value's trigger, which rune-kit keys after the Select.
fn anchor(key: &str) -> String {
    format!("{key}-trigger")
}

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

fn position_title(position: Position) -> &'static str {
    match position {
        Position::Center => "Center",
        Position::Top => "Top",
    }
}

fn display_option(display: DisplaySelection) -> (&'static str, &'static str) {
    match display {
        DisplaySelection::Mouse => ("mouse", "Mouse pointer"),
        DisplaySelection::Main => ("main", "Main display"),
        DisplaySelection::Active => ("active", "Focused window"),
    }
}

fn input_source_title(source: &InputSource) -> String {
    match source {
        InputSource::Ascii => "ASCII".to_owned(),
        InputSource::Keep => "Keep".to_owned(),
        InputSource::Id(id) => id.rsplit('.').next().unwrap_or(id).to_owned(),
    }
}

/// Open the shaders folder in Finder, for a user to drop a file in. Settings makes it first
/// only while it may write beside config.toml, checked now: something else may have taken the
/// file over since the window opened.
fn open_shaders_folder() {
    std::thread::spawn(|| {
        let dir = crate::shader::shaders_dir();
        if !crate::settings::read_only()
            && let Err(e) = std::fs::create_dir_all(&dir)
        {
            crate::log::warn(&format!("couldn't create {}: {e}", dir.display()));
            return;
        }
        if dir.is_dir() {
            _ = dupin_mac::workspace::open(&dir);
        }
    });
}

/// The example a `backdrop` path is Settings' copy of.
fn example_of(path: &str) -> Option<&'static str> {
    EXAMPLES
        .iter()
        .map(|(name, _, _)| *name)
        .find(|name| crate::shader::example_pick(name) == path)
}

fn backdrop_title(backdrop: &Backdrop) -> String {
    match backdrop {
        Backdrop::Flow => "Flow".to_owned(),
        Backdrop::Off => "Off".to_owned(),
        Backdrop::File(path) => match example_of(path) {
            Some(name) => EXAMPLES
                .iter()
                .find(|(n, _, _)| *n == name)
                .map(|(_, t, _)| (*t).to_owned())
                .unwrap_or_default(),
            None => {
                let name = path.rsplit('/').next().unwrap_or(path);
                name.strip_suffix(".wgsl").unwrap_or(name).to_owned()
            }
        },
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
    permissions: Vec<Access>,
    /// The first-run welcome's step while it covers the pages.
    welcome: Option<welcome::Step>,
    pub(crate) hotkey_problem: Option<String>,
    pub(crate) scripts_notice: Option<String>,
    preview_dark: Option<bool>,
    active_radius: u16,
}

struct LoginPreference {
    item: LoginItem,
    status: LoginStatus,
    error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LoginAction {
    SetEnabled(bool),
    OpenSettings,
}

fn login_action(status: LoginStatus) -> Option<LoginAction> {
    match status {
        LoginStatus::NotRegistered | LoginStatus::NotFound => Some(LoginAction::SetEnabled(true)),
        LoginStatus::Enabled => Some(LoginAction::SetEnabled(false)),
        LoginStatus::RequiresApproval => Some(LoginAction::OpenSettings),
        _ => None,
    }
}

impl Prefs {
    pub fn new(
        current: Rc<RefCell<Config>>,
        binding: Arc<Binding>,
        panel: AppHandle<Ui>,
        updater: Updater,
        settings: Arc<crate::settings::Service>,
        welcome: bool,
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
            hotkey_problem: None,
            scripts_notice: None,
            login: objc2::MainThreadMarker::new()
                .and_then(LoginItem::for_current_app)
                .map(|item| LoginPreference {
                    status: item.status(),
                    item,
                    error: None,
                }),
            permissions: Vec::new(),
            welcome: welcome.then_some(welcome::Step::Hotkey),
        }
    }

    pub fn refresh_login_item(&mut self) {
        if let Some(login) = &mut self.login {
            login.status = login.item.status();
            login.error = None;
        }
    }

    pub fn refresh_permissions(&mut self) {
        if self.welcome.is_none() && self.page == Page::Permissions {
            self.permissions = permissions::read();
        }
    }

    fn show(&mut self, page: Page) {
        self.welcome = None;
        self.page = page;
        self.refresh_permissions();
    }

    fn change_login_item(&mut self, action: LoginAction) {
        self.refresh_login_item();
        let Some(login) = &mut self.login else {
            return;
        };
        // System Settings can change the item between rendering the button and pressing it.
        if login_action(login.status) != Some(action) {
            return;
        }
        match action {
            LoginAction::SetEnabled(enabled) => {
                login.error = login.item.set_enabled(enabled).err();
                login.status = login.item.status();
            }
            LoginAction::OpenSettings => login.item.open_settings(),
        }
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

    /// The backdrop menu: the built-in shader, none, an example (copied beside config.toml to
    /// be edited) or a file in the shaders folder. A file elsewhere that config.toml names shows
    /// checked with them.
    fn pick_backdrop(&mut self, tx: &mut Tx) {
        let current = &self.config.backdrop;
        let id = |b: &Backdrop| match b {
            Backdrop::Flow => FLOW.to_owned(),
            Backdrop::Off => OFF.to_owned(),
            Backdrop::File(path) => match example_of(path) {
                Some(name) => format!("{EXAMPLE}{name}"),
                None => format!("{FILE}{path}"),
            },
        };
        let selected = id(current);
        let item = |b: Backdrop| {
            let id = id(&b);
            PopupItem::new(id.clone(), backdrop_title(&b)).checked(id == selected)
        };
        let mut files = crate::shader::user_shaders();
        if let Backdrop::File(path) = current
            && example_of(path).is_none()
            && !files.contains(path)
        {
            files.push(path.clone());
        }
        let mut menu = PopupMenu::new()
            .item(item(Backdrop::Flow))
            .item(item(Backdrop::Off))
            .separator()
            .items(
                EXAMPLES
                    .iter()
                    .map(|(name, _, _)| item(Backdrop::File(crate::shader::example_pick(name)))),
            );
        if !files.is_empty() {
            menu = menu
                .separator()
                .items(files.into_iter().map(|f| item(Backdrop::File(f))));
        }
        tx.popup_menu(
            menu.selected(selected),
            anchor(BACKDROP),
            |s: &mut Prefs, picked, _| {
                let Some(picked) = picked else { return };
                let next = if picked == FLOW {
                    Backdrop::Flow
                } else if picked == OFF {
                    Backdrop::Off
                } else if let Some(name) = picked.strip_prefix(EXAMPLE) {
                    match crate::shader::install_example(name) {
                        Ok(next) => next,
                        Err(e) => return crate::log::warn(&format!("couldn't copy the {name} shader: {e}")),
                    }
                } else if let Some(path) = picked.strip_prefix(FILE) {
                    Backdrop::File(path.to_owned())
                } else {
                    return;
                };
                s.set(|c| c.backdrop = next);
            },
        );
    }

    /// Open a menu of `options` (id, title) over the row's value, the current one checked, and
    /// apply the pick.
    fn choose(
        tx: &mut Tx,
        key: &str,
        options: Vec<(String, String)>,
        selected: &str,
        apply: impl FnOnce(&mut Prefs, &str) + 'static,
    ) {
        tx.popup_menu(
            PopupMenu::picker(options, selected),
            anchor(key),
            move |s: &mut Prefs, picked, _| {
                if let Some(id) = picked {
                    apply(s, id);
                }
            },
        );
    }

    fn pick_display(&mut self, tx: &mut Tx) {
        const CHOICES: [DisplaySelection; 3] = [
            DisplaySelection::Mouse,
            DisplaySelection::Main,
            DisplaySelection::Active,
        ];
        let options = CHOICES
            .into_iter()
            .map(display_option)
            .map(|(id, label)| (id.to_owned(), label.to_owned()))
            .collect();
        Prefs::choose(
            tx,
            DISPLAY,
            options,
            display_option(self.config.display).0,
            |s, id| {
                if let Some(value) = CHOICES.into_iter().find(|value| display_option(*value).0 == id) {
                    s.set(|config| config.display = value);
                }
            },
        );
    }

    fn pick_position(&mut self, tx: &mut Tx) {
        let options = [(Position::Center, "center"), (Position::Top, "top")]
            .map(|(p, id)| (id.to_owned(), position_title(p).to_owned()))
            .to_vec();
        let selected = match self.config.position {
            Position::Center => "center",
            Position::Top => "top",
        };
        Prefs::choose(tx, POSITION, options, selected, |s, id| {
            s.set(|c| {
                c.position = if id == "top" {
                    Position::Top
                } else {
                    Position::Center
                }
            });
        });
    }

    fn pick_input_source(&mut self, tx: &mut Tx) {
        let current = &self.config.input_source;
        let mut options = vec![
            ("ascii".to_owned(), input_source_title(&InputSource::Ascii)),
            ("keep".to_owned(), input_source_title(&InputSource::Keep)),
        ];
        // A source picked by identifier in config.toml stays pickable.
        if let InputSource::Id(_) = current {
            options.push(("id".to_owned(), input_source_title(current)));
        }
        let selected = match current {
            InputSource::Ascii => "ascii",
            InputSource::Keep => "keep",
            InputSource::Id(_) => "id",
        };
        Prefs::choose(tx, INPUT_SOURCE, options, selected, |s, id| match id {
            "ascii" => s.set(|c| c.input_source = InputSource::Ascii),
            "keep" => s.set(|c| c.input_source = InputSource::Keep),
            _ => {}
        });
    }

    fn pick_file_limit(&mut self, tx: &mut Tx) {
        let options = FILE_LIMITS
            .iter()
            .map(|n| (n.to_string(), n.to_string()))
            .collect();
        let selected = self.config.file_limit.to_string();
        Prefs::choose(tx, FILE_LIMIT, options, &selected, |s, id| {
            if let Ok(n) = id.parse() {
                s.set(|c| c.file_limit = n);
            }
        });
    }

    /// Persist the settings and hand them to the panel.
    fn save(&mut self, base: &Config) {
        *self.current.borrow_mut() = self.config.clone();
        // The panel must see this edit before the worker can echo its saved file back.
        self.publish();
        if let Err(error) = self.settings.save(base, &self.config) {
            self.set_problem(Some(error));
        }
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
        (None, Page::General) => general(s, k),
        (None, Page::Search) => search(s, k),
        (None, Page::Appearance) => appearance(s, k, cx.system_dark()),
        (None, Page::Permissions) => permissions::page(s, k, hair),
        (None, Page::Updates) => updates(s, k),
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
                .children(
                    if s.welcome.is_some() || matches!(s.page, Page::Appearance | Page::Permissions) {
                        rows
                    } else {
                        vec![card(k, hair, rows)]
                    },
                )
                .children(
                    (s.welcome.is_none() && s.page == Page::General && !s.config.bindings.is_empty()).then(
                        || {
                            col()
                                .key("configured-hotkeys")
                                .gap(4.0)
                                .child(
                                    text("Configured hotkeys")
                                        .mx(ROW_PX)
                                        .font(k.ty.body_medium)
                                        .color(k.c.text_base),
                                )
                                .child(card(k, hair, configured_hotkeys(s, k)))
                        },
                    ),
                )
                .children(
                    s.scripts_notice
                        .as_ref()
                        .filter(|_| s.welcome.is_none() && s.page == Page::Search)
                        .map(|notice| {
                            col()
                                .key("scripts-notice")
                                .mx(ROW_PX)
                                .gap(4.0)
                                .child(
                                    text("Script discovery")
                                        .font(k.ty.body_medium)
                                        .color(k.c.text_base),
                                )
                                .child(
                                    text(notice)
                                        .w_full()
                                        .font(DESCRIPTION)
                                        .color(k.c.text_muted)
                                        .wrap(),
                                )
                        }),
                )
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
            .px(6.0)
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
        .children(s.updater.available().then(|| update_footer(s, k)))
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
        .px(6.0)
        .self_start()
        .center_y()
        .radius(k.radius.sm)
        .hover(c.hover)
        .press(c.pressed)
        .child(text(label.to_owned()).font(k.ty.small).color(c.text_muted))
}

fn update_footer(s: &Prefs, k: &Kit) -> El {
    let status = s.updater.status();
    let title = match &status {
        Status::Off => "Update settings",
        Status::Checking => "Checking…",
        Status::Downloading => "Downloading…",
        Status::Ready(_) => "Update ready",
        Status::Download { .. } => "Update available",
        _ => "Check for Updates",
    };
    let version = if s.updater.version().is_empty() {
        "Updates".to_owned()
    } else {
        format!("Version {}", s.updater.version())
    };
    col()
        .key("update-footer")
        .w_full()
        .mt(12.0)
        .child(
            text(version.clone())
                .key("current-version")
                .role("button")
                .label(format!("{version}. Open update settings"))
                .focusable(true)
                .focus_selected(s.page == Page::Updates)
                .focus_ring(2.0, 0.0, k.c.border_focus)
                .on_press(|s: &mut Prefs| s.page = Page::Updates)
                .w_full()
                .px(6.0)
                .font(k.ty.small.lh(20.0))
                .color(k.c.text_muted)
                .truncate(),
        )
        .child(
            row()
                .key(Page::Updates.key())
                .role("button")
                .label(title)
                .focusable(true)
                .focus_ring(2.0, 0.0, k.c.border_focus)
                .w_full()
                .h(24.0)
                .px(6.0)
                .center_y()
                .radius(k.radius.sm)
                .hover(k.c.hover)
                .press(k.c.pressed)
                .on_press(|s: &mut Prefs| {
                    s.page = Page::Updates;
                    let status = s.updater.status();
                    if status.can_check() && !matches!(status, Status::Download { .. }) {
                        s.updater.check_now();
                    }
                })
                .child(text(title).font(k.ty.small).color(k.c.text_muted).truncate()),
        )
}

fn shortcut_recorder(s: &Prefs, k: &Kit, kind: LauncherShortcut, key: &str, label: &str) -> El {
    let i = kind as usize;
    let shortcut_problem = s.shortcut_problems[i].clone();
    let refusal = move |error: &ShortcutError, chord: Option<&Shortcut>| {
        shortcut_problem
            .as_ref()
            .filter(|_| *error == ShortcutError::Conflict)
            .cloned()
            .unwrap_or_else(|| shortcut_error_message(error, chord))
    };
    ShortcutRecorder::new(key, &s.recorders[i], move |s: &mut Prefs| &mut s.recorders[i])
        .label(label)
        .width(180.0)
        .on_change(move |s: &mut Prefs, next, _| s.change_hotkey(kind, next))
        .error_message(refusal)
        .disabled(s.read_only)
        .build(k)
}

/// The panel hotkey's row, on General and in the welcome.
fn hotkey_setting(s: &Prefs, k: &Kit) -> El {
    let kind = LauncherShortcut::Panel;
    let shortcut = s.recorders[kind as usize].shortcut();
    let detail = match (s.binding.launch_problem(kind, shortcut), shortcut) {
        (Some(crate::hotkey::LaunchProblem::Occupied(owner)), Some(chord)) => format!(
            "{owner} already uses {}, so choose another or free it there",
            chord.keycaps().join("")
        ),
        (Some(_), Some(chord)) => format!(
            "Dupin couldn't register {}, so choose another",
            chord.keycaps().join("")
        ),
        _ => "Shows the panel from any app".to_owned(),
    };
    setting(
        k,
        &icons::KEYBOARD,
        "Hotkey",
        Some(&detail),
        shortcut_recorder(s, k, kind, "hotkey", "Show Dupin"),
    )
}

fn general(s: &Prefs, k: &Kit) -> Vec<El> {
    let display = menu_value(
        s,
        k,
        DISPLAY,
        "Display",
        display_option(s.config.display).1,
        Prefs::pick_display,
    );
    let position = menu_value(
        s,
        k,
        POSITION,
        "Position",
        position_title(s.config.position),
        Prefs::pick_position,
    );
    let input_source = menu_value(
        s,
        k,
        INPUT_SOURCE,
        "Input source",
        &input_source_title(&s.config.input_source),
        Prefs::pick_input_source,
    );
    let relaunch = value_button(k, "relaunch", "Relaunch", &icons::REFRESH).on_press(|s: &mut Prefs| {
        s.panel.update(|ui, tx| ui.relaunch(tx));
    });
    let automation_enabled = s.config.system_automation;
    let automation = Switch::new(automation_enabled)
        .label("System Command Automation")
        .hide_label()
        .disabled(s.read_only)
        .on_toggle(move |s: &mut Prefs| s.set(|c| c.system_automation = !automation_enabled))
        .build(k);
    vec![
        hotkey_setting(s, k),
        setting(
            k,
            &icons::KEYBOARD,
            "Selection hotkey",
            Some("Searches selected text. Requires Accessibility access."),
            shortcut_recorder(
                s,
                k,
                LauncherShortcut::Selection,
                "selection-hotkey",
                "Search selected text",
            ),
        ),
        setting(
            k,
            &icons::PANEL,
            "Display",
            Some(if s.config.display == DisplaySelection::Active {
                "Uses the focused window's display when Accessibility access allows"
            } else {
                "Which display the panel opens on"
            }),
            display,
        ),
        setting(
            k,
            &icons::PANEL,
            "Position",
            Some("Where on the display the panel opens"),
            position,
        ),
        setting(
            k,
            &icons::LANGUAGES,
            "Input source",
            Some("ASCII switches an input method to Roman while the panel shows"),
            input_source,
        ),
        login_setting(s.login.as_ref(), k),
        setting(
            k,
            &rune_theme_opencode::icons::SHIELD,
            "System Command Automation",
            Some("Show system commands that may ask for macOS Automation when confirmed"),
            automation,
        ),
        setting(
            k,
            &icons::REFRESH,
            "Relaunch Dupin",
            Some("Restart, reload configuration and cancel running scripts"),
            relaunch,
        ),
    ]
}

fn configured_hotkeys(s: &Prefs, k: &Kit) -> Vec<El> {
    use dupin_core::config::HotkeyTarget;

    s.config
        .bindings
        .iter()
        .map(|(chord, target)| {
            let keys = chord
                .parse::<Shortcut>()
                .map_or_else(|_| chord.clone(), |shortcut| shortcut.keycaps().join(""));
            let (kind, name) = match target {
                HotkeyTarget::App { bundle_id, .. } => ("App", bundle_id),
                HotkeyTarget::Command { command } => ("Command", command),
                HotkeyTarget::Script { script, .. } => ("Script", script),
            };
            setting(
                k,
                &icons::KEYBOARD,
                kind,
                Some(name),
                text(keys).font(k.ty.ui).color(k.c.text_base),
            )
            .key(chord)
        })
        .collect()
}

fn login_setting(login: Option<&LoginPreference>, k: &Kit) -> El {
    let status = login.map(|login| login.status);
    let detail = match status {
        Some(LoginStatus::NotRegistered | LoginStatus::Enabled) => "Open Dupin when you log in",
        Some(LoginStatus::RequiresApproval) => "Approval required in System Settings",
        Some(LoginStatus::NotFound) => "Login item not found",
        Some(_) => "macOS returned an unknown login item status",
        None => "Requires a signed Dupin.app outside the Nix store",
    };
    let detail = match login.and_then(|login| login.error.as_deref()) {
        Some(error) => format!("{detail}. {error}"),
        None => detail.to_owned(),
    };
    let control = match status.and_then(login_action) {
        Some(LoginAction::OpenSettings) => Button::new("Open Login Items")
            .on_press(|s: &mut Prefs| s.change_login_item(LoginAction::OpenSettings))
            .build(k),
        action => {
            let enabled = action == Some(LoginAction::SetEnabled(false));
            Switch::new(enabled)
                .label("Launch at login")
                .hide_label()
                .disabled(action.is_none())
                .on_toggle(move |s: &mut Prefs| s.change_login_item(LoginAction::SetEnabled(!enabled)))
                .build(k)
        }
    };
    setting(
        k,
        &icons::LOGIN,
        "Launch at login",
        Some(&detail),
        control.key("launch-at-login"),
    )
}

fn search(s: &Prefs, k: &Kit) -> Vec<El> {
    let show_recent = !s.config.show_recent;
    let recent = Switch::new(s.config.show_recent)
        .label("Show recent items")
        .hide_label()
        .disabled(s.read_only)
        .on_toggle(move |s: &mut Prefs| s.set(|c| c.show_recent = show_recent))
        .build(k);
    let limit = menu_value(
        s,
        k,
        FILE_LIMIT,
        "Files shown",
        &s.config.file_limit.to_string(),
        Prefs::pick_file_limit,
    );
    let calendar_enabled = !s.config.calendar.enabled;
    let calendar = Switch::new(s.config.calendar.enabled)
        .label("Calendar search")
        .hide_label()
        .disabled(s.read_only)
        .on_toggle(move |s: &mut Prefs| s.set(|c| c.calendar.enabled = calendar_enabled))
        .build(k);
    let next_clipboard = !s.config.clipboard.enabled;
    let clipboard = Switch::new(s.config.clipboard.enabled)
        .label("Clipboard history")
        .hide_label()
        .disabled(s.read_only)
        .on_toggle(move |s: &mut Prefs| s.set(|c| c.clipboard.enabled = next_clipboard))
        .build(k);
    let next_paste = !s.config.snippet_paste;
    let paste = Switch::new(s.config.snippet_paste)
        .label("Paste snippets")
        .hide_label()
        .disabled(s.read_only)
        .on_toggle(move |s: &mut Prefs| s.set(|c| c.snippet_paste = next_paste))
        .build(k);
    let enabled = s.config.window_management;
    let window_management = Switch::new(enabled)
        .label("Window Management")
        .hide_label()
        .disabled(s.read_only)
        .on_toggle(move |s: &mut Prefs| s.set(|c| c.window_management = !enabled))
        .build(k);
    let next_calculator = !s.config.calculator;
    let calculator = Switch::new(s.config.calculator)
        .label("Calculator")
        .hide_label()
        .disabled(s.read_only)
        .on_toggle(move |s: &mut Prefs| s.set(|c| c.calculator = next_calculator))
        .build(k);
    let next_currency = !s.config.calculator_currency;
    let currency = Switch::new(s.config.calculator_currency)
        .label("Currency conversion")
        .hide_label()
        .disabled(s.read_only || !s.config.calculator)
        .on_toggle(move |s: &mut Prefs| s.set(|c| c.calculator_currency = next_currency))
        .build(k);
    let mut rows = vec![
        setting(
            k,
            &icons::CALCULATOR,
            "Calculator",
            Some("Show calculations and unit conversions in app search"),
            calculator,
        ),
        setting(
            k,
            &icons::CALCULATOR,
            "Currency conversion",
            Some("Fetch European Central Bank rates in the background"),
            currency,
        ),
        setting(
            k,
            &icons::PANEL,
            "Show recent items",
            Some("Show recently and frequently used apps and files when the search is empty"),
            recent,
        ),
        setting(
            k,
            &icons::FILES,
            "Files shown",
            Some("The most files a Spotlight search lists"),
            limit,
        ),
        setting(
            k,
            &icons::SEARCH,
            "Calendar search",
            Some("Type cal then Space to search events"),
            calendar,
        ),
        setting(
            k,
            &icons::CLIPBOARD,
            "Clipboard history",
            Some("Keeps what you copy in memory until Dupin quits; type clip and Space to search it"),
            clipboard,
        ),
        setting(
            k,
            &icons::KEYBOARD,
            "Paste snippets",
            Some("Enter pastes a snippet into the front app instead of only copying it"),
            paste,
        ),
        setting(
            k,
            &icons::PANEL,
            "Window Management",
            Some("Show commands to move and resize windows. Requires Accessibility on first use."),
            window_management,
        ),
    ];
    // Checking doesn't ask macOS for access; only pasting a snippet does.
    if s.config.snippet_paste && !dupin_mac::paste::allowed() {
        let open = value_button(
            k,
            "open-accessibility",
            "Open",
            &rune_theme_opencode::icons::CHEVRON_RIGHT,
        )
        .on_press(|_: &mut Prefs| {
            std::thread::spawn(|| {
                let action =
                    dupin_mac::workspace::Action::OpenUrl(dupin_mac::accessibility::SETTINGS_URL.into());
                if let Err(error) = dupin_mac::workspace::run(&action) {
                    crate::log::warn(&error);
                }
            });
        });
        rows.push(setting(
            k,
            &icons::INFO,
            "Accessibility access",
            Some("Pasting needs Dupin allowed in Privacy & Security › Accessibility"),
            open,
        ));
    }
    rows
}

use dupin_core::config::{Colors, FontFamily, Theme};

type Palette = (&'static str, &'static str, &'static str);

const PRESETS: [(&str, Option<Palette>); 3] = [
    ("Default", None),
    ("Ocean", Some(("#E9F4FA", "#142734", "#176EA6"))),
    ("Rose", Some(("#FFF0F5", "#30202A", "#A52D61"))),
];

fn preset_colors(i: usize) -> (Colors, Colors) {
    let Some((light, dark, accent)) = PRESETS[i].1 else {
        return (Colors::default(), Colors::default());
    };
    let colors = |bg: &str| Colors {
        background: Some(bg.into()),
        accent: Some(accent.into()),
        ..Colors::default()
    };
    (colors(light), colors(dark))
}

fn color_preset(t: &Theme) -> Option<usize> {
    (0..PRESETS.len()).find(|&i| {
        let (light, dark) = preset_colors(i);
        t.light == light && t.dark == dark
    })
}

fn set_font(t: &mut Theme, size: u16) {
    t.font_size = size;
    if size == 18 {
        t.row_height = t.row_height.max(61);
    }
}

fn set_rows(t: &mut Theme, height: u16) {
    t.row_height = height;
    if height == 52 {
        t.font_size = t.font_size.min(15);
    }
}

fn segments(
    s: &Prefs,
    k: &Kit,
    key: &'static str,
    label: &'static str,
    labels: &[&str],
    selected: Option<usize>,
    apply: impl Fn(&mut Theme, usize) + Clone + 'static,
) -> El {
    let mut control = SegmentedControl::new(labels, selected).label(label).fit();
    if s.read_only {
        for i in 0..labels.len() {
            control = control.segment_state(i, State::Disabled);
        }
    } else {
        control = control.on_pick(move |s: &mut Prefs, i, _| {
            let apply = apply.clone();
            s.set(move |c| apply(&mut c.theme, i));
        });
    }
    control.build(k).key(key)
}

fn theme_row(k: &Kit, name: &str, controls: El) -> El {
    row()
        .w_full()
        .h(40.0)
        .px(ROW_PX)
        .center_y()
        .gap(12.0)
        .child(text(name).font(k.ty.small).color(k.c.text_base).grow(1.0))
        .child(controls)
}

fn theme_card(s: &Prefs, k: &Kit, dark: bool) -> El {
    let t = &s.config.theme;
    let selected = color_preset(t);
    let mut swatches = row()
        .role("radiogroup")
        .label("Palette")
        .focus_group()
        .gap(12.0)
        .center_y();
    for (i, (name, _)) in PRESETS.iter().enumerate() {
        let (light, dark) = preset_colors(i);
        let sample = Theme {
            light,
            dark,
            ..Theme::default()
        };
        let palette = crate::view::ResolvedTheme::new(&sample, false);
        let mut swatch = el()
            .key(format!("color-{i}"))
            .label(*name)
            .role("radio")
            .focusable(!s.read_only)
            .focus_selected(selected == Some(i))
            .checked(selected == Some(i))
            .position_in_set(i + 1, PRESETS.len())
            .size(26.0, 26.0)
            .radius(13.0)
            .padding(3.0)
            .border(
                2.0,
                if selected == Some(i) {
                    k.c.border_focus
                } else {
                    rune::Color::TRANSPARENT
                },
            )
            .child(
                el().size(20.0, 20.0)
                    .radius(10.0)
                    .fill(palette.card)
                    .center()
                    .child(el().size(9.0, 9.0).radius(4.5).fill(palette.accent)),
            );
        if s.read_only {
            swatch = swatch.disabled(true).layer_opacity(0.5);
        } else {
            swatch = swatch
                .focus_ring(2.0, 2.0, k.c.border_focus)
                .on_press(move |s: &mut Prefs| {
                    s.set(|c| {
                        (c.theme.light, c.theme.dark) = preset_colors(i);
                    })
                });
        }
        swatches = swatches.child(swatch);
    }
    if selected.is_none() {
        swatches = swatches.child(
            text("Custom")
                .font(k.ty.small)
                .color(k.c.text_muted)
                .px(8.0)
                .py(4.0)
                .radius(6.0)
                .border(1.0, k.c.border_muted),
        );
    }
    let toggle = SegmentedControl::new(
        &["Light", "Dark"],
        Some(usize::from(s.preview_dark.unwrap_or(dark))),
    )
    .label("Preview appearance")
    .fit()
    .on_pick(|s: &mut Prefs, i, _| s.preview_dark = Some(i == 1))
    .build(k)
    .key("preview-appearance");
    let well = col()
        .w_full()
        .h(156.0)
        .padding(10.0)
        .gap(6.0)
        .radius(k.radius.md)
        .fill(k.c.layer_01)
        .child(
            row()
                .w_full()
                .h(28.0)
                .center_y()
                .child(
                    text(format!(
                        "{} × {} · {} pt · {} pt corners",
                        t.panel_width, t.row_height, t.font_size, t.corner_radius
                    ))
                    .font(k.ty.small)
                    .color(k.c.text_muted)
                    .grow(1.0),
                )
                .child(toggle),
        )
        .child(el().w_full().grow(1.0).center().child(crate::view::theme_preview(
            t,
            s.preview_dark.unwrap_or(dark),
            414.0,
        )));
    let mut reset = value_button(k, "reset-theme", "Reset", &icons::REFRESH);
    if s.read_only || *t == Theme::default() {
        reset = reset.disabled(true).layer_opacity(0.5);
    } else {
        reset = reset.on_press(|s: &mut Prefs| s.set(|c| c.theme = Theme::default()));
    }
    let mut corners = row().gap(10.0).center_y();
    if t.corner_radius != s.active_radius {
        corners = corners.child(
            text("Relaunch to apply")
                .font(TextStyle::new(11.0, 440.0))
                .color(k.c.text_muted),
        );
    }
    corners = corners.child(segments(
        s,
        k,
        "theme-corners",
        "Corners",
        &["Square", "14", "28"],
        [0, 14, 28].iter().position(|&v| v == t.corner_radius),
        |t, i| t.corner_radius = [0, 14, 28][i],
    ));
    card(
        k,
        1.0,
        vec![
            row()
                .w_full()
                .h(40.0)
                .px(ROW_PX)
                .gap(8.0)
                .center_y()
                .child(icon(&icons::SPARKLES, 16.0).color(k.c.text_muted))
                .child(text("Theme").font(k.ty.small).color(k.c.text_base).grow(1.0)),
            el().w_full().px(ROW_PX).mb(4.0).child(well),
            theme_row(k, "Palette", swatches),
            theme_row(
                k,
                "Font",
                row()
                    .gap(8.0)
                    .child(segments(
                        s,
                        k,
                        "theme-font",
                        "Font",
                        &["System", "Mono"],
                        Some(usize::from(t.font_family == FontFamily::Monospace)),
                        |t, i| t.font_family = [FontFamily::System, FontFamily::Monospace][i],
                    ))
                    .child(segments(
                        s,
                        k,
                        "theme-size",
                        "Font size",
                        &["15", "18"],
                        [15, 18].iter().position(|&v| v == t.font_size),
                        |t, i| set_font(t, [15, 18][i]),
                    )),
            ),
            theme_row(
                k,
                "Width",
                segments(
                    s,
                    k,
                    "theme-width",
                    "Width",
                    &["Narrow", "Regular", "Wide"],
                    [400, 680, 900].iter().position(|&v| v == t.panel_width),
                    |t, i| t.panel_width = [400, 680, 900][i],
                ),
            ),
            theme_row(
                k,
                "Rows",
                segments(
                    s,
                    k,
                    "theme-rows",
                    "Rows",
                    &["Compact", "Comfy", "Spacious"],
                    [52, 72, 100].iter().position(|&v| v == t.row_height),
                    |t, i| set_rows(t, [52, 72, 100][i]),
                ),
            ),
            theme_row(k, "Corners", corners),
        ],
    )
    .child(reset.pin(None, Some(6.0), Some(ROW_PX), None))
}

fn appearance(s: &Prefs, k: &Kit, dark: bool) -> Vec<El> {
    // The menu lists the shaders folder as it stands when it opens, so the trigger shows only
    // the current choice.
    let backdrop = menu_value(
        s,
        k,
        BACKDROP,
        "Backdrop",
        &backdrop_title(&s.config.backdrop),
        Prefs::pick_backdrop,
    );
    let dir = crate::shader::shaders_dir();
    let folder = tilde(&dir.display().to_string());
    let open = value_button(
        k,
        "open-shaders",
        "Open",
        &rune_theme_opencode::icons::CHEVRON_RIGHT,
    );
    let open = if !s.read_only || dir.is_dir() {
        open.on_press(|_: &mut Prefs| open_shaders_folder())
    } else {
        open.disabled(true).layer_opacity(0.5)
    };
    vec![
        theme_card(s, k, dark),
        card(
            k,
            1.0,
            vec![
                setting(
                    k,
                    &icons::SPARKLES,
                    "Backdrop",
                    Some("The shader that animates behind the panel"),
                    backdrop,
                ),
                setting(
                    k,
                    &icons::FOLDER,
                    "Shaders folder",
                    Some(&format!("Put .wgsl files in {folder} to pick them above")),
                    open,
                ),
            ],
        ),
    ]
}

/// Whether to install updates by themselves, and where the updater is: the version, what it
/// is doing, and the one thing to do next.
fn updates(s: &Prefs, k: &Kit) -> Vec<El> {
    let next_auto_update = !s.config.auto_update;
    let auto = Switch::new(s.config.auto_update)
        .label("Install updates automatically")
        .hide_label()
        .disabled(s.read_only)
        .on_toggle(move |s: &mut Prefs| s.set(|c| c.auto_update = next_auto_update))
        .build(k);
    let status = s.updater.status();
    let action = match &status {
        Status::Ready(_) => Button::new("Restart to Update").on_press(|s: &mut Prefs| {
            s.panel.update(|ui, tx| ui.relaunch(tx));
        }),
        Status::Download { .. } => {
            Button::new("Download").on_press(|s: &mut Prefs| s.updater.open_download())
        }
        _ => Button::new("Check for Updates")
            .disabled(!status.can_check())
            .on_press(|s: &mut Prefs| s.updater.check_now()),
    }
    .build(k);
    let mut line = status.text(s.config.auto_update, s.updater.download_only());
    if matches!(status, Status::Ready(_)) {
        line.push_str(". Restarting cancels running scripts.");
    }
    vec![
        setting(
            k,
            &icons::REFRESH,
            "Install updates automatically",
            Some("New versions download in the background and install on restart"),
            auto,
        ),
        setting(
            k,
            &icons::INFO,
            &format!("Dupin {}", s.updater.version()),
            Some(&line),
            action,
        ),
    ]
}

/// A row's current value and a chevron, which `open` turns into a menu of the others. Locked
/// while config.toml is read-only.
fn menu_value(s: &Prefs, k: &Kit, key: &str, name: &str, value: &str, open: fn(&mut Prefs, &mut Tx)) -> El {
    let trigger = Select::new(key, &[value])
        .appearance(SelectAppearance::Inline)
        .label(name)
        .current(Some(value))
        .disabled(s.read_only)
        .trigger(k)
        .max_w(VALUE_MAX_W)
        .mr(-8.0);
    if s.read_only {
        trigger
    } else {
        trigger.on_press_tx(open)
    }
}

/// A row's action: a label and an icon, quiet until hovered.
fn value_button(k: &Kit, key: &str, label: &str, glyph: &'static Icon) -> El {
    let c = &k.c;
    row()
        .key(key)
        .role("button")
        .label(label)
        .focusable(true)
        .focus_ring(2.0, 0.0, c.border_focus)
        .h(24.0)
        .pl(8.0)
        .pr(4.0)
        .mr(-9.0)
        .gap(4.0)
        .center_y()
        .radius(k.radius.sm)
        .hover(c.hover)
        .press(c.pressed)
        .child(text(label).font(k.ty.ui_medium.lh(16.0)).color(c.text_base))
        .child(icon(glyph, 16.0).color(c.icon_muted))
}

/// Rows on one raised card, a hairline between each.
fn card(k: &Kit, hair: f32, rows: Vec<El>) -> El {
    let count = rows.len();
    let mut card = col().key("card").w_full().radius(k.radius.lg).fill(k.c.bg_base);
    for shadow in &k.elevation.raised {
        card = card.box_shadow(*shadow);
    }
    for (i, r) in rows.into_iter().enumerate() {
        card = card.child(r);
        if i + 1 < count {
            card = card.child(
                el().key(format!("sep{i}"))
                    .h(hair)
                    .mx(ROW_PX)
                    .fill(k.c.border_muted)
                    .a11y_hidden(),
            );
        }
    }
    card
}

/// A muted remark under a page's cards.
fn note(k: &Kit, words: &str) -> El {
    text(words.to_owned())
        .font(DESCRIPTION)
        .color(k.c.text_muted)
        .mx(ROW_PX)
        .wrap()
}

/// `$HOME/x` as `~/x`.
fn tilde(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => path
            .strip_prefix(&home)
            .map_or_else(|| path.to_owned(), |rest| format!("~{rest}")),
        _ => path.to_owned(),
    }
}

/// A row: an icon, the setting's name and what it does, and its control at the trailing edge.
fn setting(k: &Kit, glyph: &'static Icon, label: &str, detail: Option<&str>, control: El) -> El {
    let mut words = col()
        .grow(1.0)
        .min_w(100.0)
        .gap(2.0)
        .child(text(label.to_owned()).font(k.ty.ui).color(k.c.text_base));
    if let Some(detail) = detail {
        words = words.child(
            text(detail.to_owned())
                .font(DESCRIPTION)
                .color(k.c.text_muted)
                .wrap(),
        );
    }
    row()
        .w_full()
        .min_h(if detail.is_some() { 60.0 } else { 48.0 })
        .px(ROW_PX)
        .py(8.0)
        .gap(12.0)
        .center_y()
        .label(label)
        .child(icon(glyph, 16.0).color(k.c.icon_base))
        .child(words)
        .child(control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn editing_stale_preferences_preserves_the_latest_external_values() {
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
        let binding = Arc::new(Binding::new(panel.clone()));
        let updater = Updater::start(false, panel.clone(), prefs);
        let mut state = Prefs::new(current.clone(), binding, panel, updater, settings.clone(), false);
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

#[cfg(test)]
mod theme_control_tests {
    use super::*;

    #[test]
    fn presets_match_only_complete_color_overrides() {
        let mut theme = Theme::default();
        assert_eq!(color_preset(&theme), Some(0));
        (theme.light, theme.dark) = preset_colors(1);
        theme.panel_width = 1200;
        assert_eq!(color_preset(&theme), Some(1));
        theme.dark.text = Some("#FFFFFF".into());
        assert_eq!(color_preset(&theme), None);
        theme = Theme::default();
        theme.light.background = Some("#123456".into());
        assert_eq!(color_preset(&theme), None);
    }

    #[test]
    fn compact_rows_and_large_fonts_remain_compatible() {
        let mut theme = Theme::default();
        set_font(&mut theme, 18);
        assert_eq!(theme.row_height, 61);
        set_rows(&mut theme, 52);
        assert_eq!(theme.font_size, 15);
        theme.row_height = 100;
        set_font(&mut theme, 18);
        assert_eq!(theme.row_height, 100);
    }
}
