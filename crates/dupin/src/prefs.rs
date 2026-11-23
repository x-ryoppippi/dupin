//! The settings window (⌘, in the panel): a sidebar of pages, each a card of settings, drawn
//! in OpenCode's theme as Hex's settings are. A release build adds a page for its updates.
//! Changes apply to the panel at once and are written back to config.toml. A read-only
//! config.toml (one Nix manages) shows its values, locked.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use dupin_core::{Backdrop, Config, InputSource, Position};
use rune::{
    AppHandle, Cx, El, Icon, PopupItem, PopupMenu, ScrollSpec, Shortcut, ShortcutError, TextStyle, Tx, col,
    el, icon, row, text,
};
use rune_kit::{
    Button, Kit, SegmentedControl, Select, SelectAppearance, ShortcutRecorder, ShortcutRecorderState, State,
    Switch, shortcut_error_message,
};

use crate::hotkey::Binding;
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
    Updates,
}

impl Page {
    fn title(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Search => "Search",
            Page::Appearance => "Appearance",
            Page::Updates => "Updates",
        }
    }

    /// The sidebar row's key, for scripts to press.
    fn key(self) -> &'static str {
        match self {
            Page::General => "page-general",
            Page::Search => "page-search",
            Page::Appearance => "page-appearance",
            Page::Updates => "page-updates",
        }
    }

    fn icon(self) -> &'static Icon {
        match self {
            Page::General => &icons::SLIDERS,
            Page::Search => &icons::SEARCH,
            Page::Appearance => &icons::SPARKLES,
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
    recorder: ShortcutRecorderState,
    /// Who holds the chord the last refusal was for, when known.
    owner: Option<String>,
    binding: Arc<Binding>,
    panel: AppHandle<Ui>,
    updater: Updater,
    read_only: bool,
    settings: Arc<crate::settings::Service>,
    config_problem: Option<String>,
    login_status: Option<dupin_mac::login_item::Status>,
    login_error: Option<String>,
    preview_dark: Option<bool>,
    active_radius: u16,
}

impl Prefs {
    pub fn new(
        current: Rc<RefCell<Config>>,
        binding: Arc<Binding>,
        panel: AppHandle<Ui>,
        updater: Updater,
        settings: Arc<crate::settings::Service>,
    ) -> Prefs {
        let config = current.borrow().clone();
        let mut recorder = ShortcutRecorderState::new(config.hotkey.parse().ok());
        if binding.failed_at_launch() {
            recorder.mark_unregistered();
        }
        let active_radius = config.theme.corner_radius;
        Prefs {
            preview_dark: None,
            active_radius,
            config,
            current,
            page: Page::General,
            recorder,
            owner: None,
            binding,
            panel,
            updater,
            read_only: crate::settings::read_only(),
            settings,
            config_problem: None,
            login_status: None,
            login_error: None,
        }
    }

    pub fn refresh_login_item(&mut self) {
        if let Some(mtm) = objc2::MainThreadMarker::new() {
            self.login_status = Some(dupin_mac::login_item::status(mtm));
            self.login_error = None;
        }
    }

    fn change_login_item(&mut self) {
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            return;
        };
        // Re-read before selecting the action in case System Settings changed since focus.
        let expected = self.login_status.as_ref().and_then(|status| status.action());
        let status = dupin_mac::login_item::status(mtm);
        if let Some(action) = status.action().filter(|action| Some(*action) == expected) {
            let (status, error) = dupin_mac::login_item::perform(action, mtm);
            self.login_status = Some(status);
            self.login_error = error;
        } else {
            self.login_status = Some(status);
            self.login_error = None;
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
        let shortcut = crate::hotkey::parse(&config.hotkey).ok().flatten();
        if self.recorder.shortcut() != shortcut.as_ref() {
            self.recorder = ShortcutRecorderState::new(shortcut);
            self.owner = None;
        }
        if self.binding.failed_at_launch() {
            self.recorder.mark_unregistered();
        }
        self.config = config;
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
        // Queue the decision so Rune publishes Settings' hidden state before it runs.
        self.panel.update(Ui::resume_restart);
    }

    fn change_hotkey(&mut self, next: Option<&Shortcut>, _: &mut Tx) -> Result<(), ShortcutError> {
        let result = self.binding.replace(next);
        self.owner = match (&result, next) {
            (Err(ShortcutError::Conflict), Some(next)) => self.binding.owner(next),
            _ => None,
        };
        result?;
        let base = self.current.borrow().clone();
        self.config = base.clone();
        self.config.hotkey = next.map(ToString::to_string).unwrap_or_default();
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
    let rows = match s.page {
        Page::General => general(s, k),
        Page::Search => search(s, k),
        Page::Appearance => appearance(s, k, cx.system_dark()),
        Page::Updates => updates(s, k),
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
                    text(s.page.title())
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
                .children(s.config_problem.as_ref().map(|problem| {
                    text(problem)
                        .key("config-problem")
                        .w_full()
                        .font(DESCRIPTION)
                        .color(k.c.text_base)
                        .wrap()
                }))
                .children(if s.page == Page::Appearance {
                    rows
                } else {
                    vec![card(k, hair, rows)]
                })
                .children(s.read_only.then(|| {
                    text("config.toml is read-only, so change these where it's managed.")
                        .font(DESCRIPTION)
                        .color(k.c.text_muted)
                        .mx(ROW_PX)
                        .wrap()
                })),
        );
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
    for page in [Page::General, Page::Search, Page::Appearance] {
        let on = s.page == page;
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
            .on_press(move |s: &mut Prefs| s.page = page)
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
    let config = row()
        .key("open-config")
        .role("button")
        .label("Open config.toml")
        .focusable(true)
        .h(20.0)
        .px(6.0)
        .self_start()
        .center_y()
        .radius(k.radius.sm)
        .hover(c.hover)
        .press(c.pressed)
        .on_press(|_: &mut Prefs| {
            if let Some(path) = crate::settings::path()
                && path.exists()
            {
                std::thread::spawn(move || dupin_mac::workspace::open(&path));
            }
        })
        .child(text("Open config.toml").font(k.ty.small).color(c.text_muted));
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
            text("Open Snapshots")
                .font(k.ty.small)
                .color(c.text_muted)
                .focusable(true)
                .on_press(|s: &mut Prefs| s.panel.update(|ui, _| ui.open_snapshots())),
        )
        .child(config)
        .children(s.updater.available().then(|| update_footer(s, k)))
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

fn general(s: &Prefs, k: &Kit) -> Vec<El> {
    let owner = s.owner.clone();
    let refusal = move |error: &ShortcutError, chord: Option<&Shortcut>| match (chord, &owner) {
        (Some(chord), Some(owner)) => format!("{} is used by {owner}.", chord.keycaps().join("")),
        _ => shortcut_error_message(error, chord),
    };
    let hotkey = ShortcutRecorder::new("hotkey", &s.recorder, |s: &mut Prefs| &mut s.recorder)
        .label("Show Dupin")
        .width(180.0)
        .on_change(Prefs::change_hotkey)
        .error_message(refusal)
        .disabled(s.read_only)
        .build(k);
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
    let login_action = s.login_status.as_ref().and_then(|status| status.action());
    let login_control = Button::new(login_action.map_or("Unavailable", |action| action.label()))
        .disabled(login_action.is_none())
        .on_press(Prefs::change_login_item)
        .build(k);
    let login_detail = s
        .login_status
        .as_ref()
        .map_or("Status unavailable", |status| status.text());
    let login_detail = match &s.login_error {
        Some(error) => format!("{login_detail}. {error}"),
        None => login_detail.to_owned(),
    };
    vec![
        setting(
            k,
            &icons::REFRESH,
            "Launch at login",
            Some(&login_detail),
            login_control,
        ),
        setting(
            k,
            &icons::KEYBOARD,
            "Hotkey",
            Some("Shows the panel from any app"),
            hotkey,
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
        setting(
            k,
            &icons::REFRESH,
            "Relaunch Dupin",
            Some("Restart and reload configuration"),
            relaunch,
        ),
    ]
}

fn search(s: &Prefs, k: &Kit) -> Vec<El> {
    let limit = menu_value(
        s,
        k,
        FILE_LIMIT,
        "Files shown",
        &s.config.file_limit.to_string(),
        Prefs::pick_file_limit,
    );
    vec![setting(
        k,
        &icons::FILES,
        "Files shown",
        Some("The most files a Spotlight search lists"),
        limit,
    )]
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
    let line = status.text(s.config.auto_update, s.updater.download_only());
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
        let mut state = Prefs::new(current.clone(), binding, panel, updater, settings.clone());
        let external = Config {
            hotkey: "ctrl+alt+k".into(),
            dictionary_keywords: vec!["lookup".into()],
            ..Config::default()
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
            state.recorder.shortcut(),
            crate::hotkey::parse(&expected.hotkey).unwrap().as_ref()
        );
        let saved = received.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(saved.result.as_ref().unwrap(), &expected);
        state.reflect_config(saved.revision, expected.clone(), false, None);
        assert_eq!(
            state.recorder.shortcut(),
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
