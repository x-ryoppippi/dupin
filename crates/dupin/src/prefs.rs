//! The settings window (⌘, in the panel): the hotkey, where the panel opens, the input source,
//! how many files a search shows, the backdrop shader and, for a release build, its updates.
//! Changes apply to the panel at once and are written back to config.toml. A read-only
//! config.toml (one Nix manages) shows its values, locked.

use std::sync::Arc;

use dupin_core::{Backdrop, Config, InputSource, Position};
use rune::{AppHandle, Cx, El, PopupItem, PopupMenu, Shortcut, ShortcutError, Tx, col, el, row, text};
use rune_kit::{
    Button, Kit, SegmentedControl, Select, SelectAppearance, ShortcutRecorder, ShortcutRecorderState, State,
    Switch, shortcut_error_message,
};

use crate::hotkey::Binding;
use crate::shader::EXAMPLES;
use crate::ui::Ui;
use crate::update::{Status, Updater};

/// The window's size, taller when it shows a release build's updates.
pub fn size(updates: bool) -> (f32, f32) {
    (560.0, if updates { 484.0 } else { 388.0 })
}

const FILE_LIMITS: [usize; 4] = [50, 100, 200, 500];
const LABEL_WIDTH: f32 = 150.0;

const BACKDROP: &str = "backdrop";
/// Where the menu opens: the Select's trigger, which Rune keys after the Select.
const BACKDROP_MENU: &str = "backdrop-trigger";
/// As wide as the segmented controls above.
const CONTROL_WIDTH: f32 = 232.0;
const FLOW: &str = "flow";
const OFF: &str = "off";
const EXAMPLE: &str = "example:";
const FILE: &str = "file:";
const OPEN_FOLDER: &str = "open-folder";

/// Open the shaders folder in Finder, making it first if need be, for a user to drop a file in.
fn open_shaders_folder() {
    std::thread::spawn(|| {
        let dir = crate::shader::shaders_dir();
        match std::fs::create_dir_all(&dir) {
            Ok(()) => _ = dupin_mac::workspace::open(&dir),
            Err(e) => crate::log::warn(&format!("couldn't create {}: {e}", dir.display())),
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
    recorder: ShortcutRecorderState,
    /// Who holds the chord the last refusal was for, when known.
    owner: Option<String>,
    binding: Arc<Binding>,
    panel: AppHandle<Ui>,
    updater: Updater,
    read_only: bool,
}

impl Prefs {
    pub fn new(config: Config, binding: Arc<Binding>, panel: AppHandle<Ui>, updater: Updater) -> Prefs {
        let mut recorder = ShortcutRecorderState::new(config.hotkey.parse().ok());
        if binding.failed_at_launch() {
            recorder.mark_unregistered();
        }
        Prefs {
            config,
            recorder,
            owner: None,
            binding,
            panel,
            updater,
            read_only: crate::settings::read_only(),
        }
    }

    fn change_hotkey(&mut self, next: Option<&Shortcut>, _: &mut Tx) -> Result<(), ShortcutError> {
        let result = self.binding.replace(next);
        self.owner = match (&result, next) {
            (Err(ShortcutError::Conflict), Some(next)) => self.binding.owner(next),
            _ => None,
        };
        result?;
        self.config.hotkey = next.map(ToString::to_string).unwrap_or_default();
        self.save();
        Ok(())
    }

    fn set(&mut self, change: impl FnOnce(&mut Config)) {
        change(&mut self.config);
        self.save();
    }

    /// The backdrop menu: the built-in shader, none, or an example, copied beside config.toml
    /// to be edited. A file config.toml names shows checked at the end.
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
        menu = menu
            .separator()
            .item(PopupItem::new(OPEN_FOLDER, "Open Shaders Folder…"));
        tx.popup_menu(
            menu.selected(selected),
            BACKDROP_MENU,
            |s: &mut Prefs, picked, _| {
                let Some(picked) = picked else { return };
                if picked == OPEN_FOLDER {
                    return open_shaders_folder();
                }
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

    /// Persist the settings and hand them to the panel.
    fn save(&self) {
        crate::settings::save(&self.config);
        let config = self.config.clone();
        self.panel.update(move |ui, _| ui.apply_config(&config));
    }
}

pub fn view(s: &Prefs, cx: &mut Cx<Prefs>) -> El {
    let k = Kit::neutral(cx.system_dark());
    let owner = s.owner.clone();
    let refusal = move |error: &ShortcutError, chord: Option<&Shortcut>| match (chord, &owner) {
        (Some(chord), Some(owner)) => format!("{} is used by {owner}.", chord.keycaps().join("")),
        _ => shortcut_error_message(error, chord),
    };
    let hotkey = ShortcutRecorder::new("hotkey", &s.recorder, |s: &mut Prefs| &mut s.recorder)
        .label("Show Dupin")
        .on_change(Prefs::change_hotkey)
        .error_message(refusal)
        .disabled(s.read_only)
        .build(&k);

    let position = segments(
        s,
        SegmentedControl::new(
            &["Center", "Top"],
            Some(match s.config.position {
                Position::Center => 0,
                Position::Top => 1,
            }),
        )
        .label("Position")
        .on_pick(|s: &mut Prefs, i, _| {
            s.set(|c| c.position = if i == 0 { Position::Center } else { Position::Top })
        }),
        2,
    )
    .build(&k);

    // A source picked by identifier in config.toml shows as a third segment.
    let mut sources = vec!["ASCII".to_owned(), "Keep".to_owned()];
    let picked = match &s.config.input_source {
        InputSource::Ascii => 0,
        InputSource::Keep => 1,
        InputSource::Id(id) => {
            sources.push(id.rsplit('.').next().unwrap_or(id).to_owned());
            2
        }
    };
    let input_source = segments(
        s,
        SegmentedControl::new(&sources, Some(picked))
            .label("Input source")
            .on_pick(|s: &mut Prefs, i, _| match i {
                0 => s.set(|c| c.input_source = InputSource::Ascii),
                1 => s.set(|c| c.input_source = InputSource::Keep),
                _ => {}
            }),
        sources.len(),
    )
    .build(&k);

    // The menu lists the shaders folder as it stands when it opens, so the Select holds only
    // the current choice.
    let title = backdrop_title(&s.config.backdrop);
    let mut backdrop = Select::new(BACKDROP, &[title.as_str()])
        .appearance(SelectAppearance::Field)
        .width(CONTROL_WIDTH)
        .label("Backdrop")
        .current(Some(&title))
        .disabled(s.read_only)
        .trigger(&k);
    if !s.read_only {
        backdrop = backdrop.on_press_tx(|s: &mut Prefs, tx| s.pick_backdrop(tx));
    }

    let limits: Vec<String> = FILE_LIMITS.iter().map(ToString::to_string).collect();
    let file_limit = segments(
        s,
        SegmentedControl::new(
            &limits,
            FILE_LIMITS.iter().position(|&n| n == s.config.file_limit),
        )
        .label("Files shown")
        .on_pick(|s: &mut Prefs, i, _| s.set(|c| c.file_limit = FILE_LIMITS[i])),
        limits.len(),
    )
    .build(&k);

    let mut path = crate::settings::path()
        .map(|p| tilde(&p.display().to_string()))
        .unwrap_or_default();
    if s.read_only {
        path.push_str(" is read-only; change it where it's managed");
    }
    let open_file = Button::new("Open config.toml")
        .build(&k)
        .on_press_tx(|_: &mut Prefs, _| {
            if let Some(path) = crate::settings::path()
                && path.exists()
            {
                std::thread::spawn(move || dupin_mac::workspace::open(&path));
            }
        });

    col()
        .key("prefs")
        .w_full()
        .h_full()
        .padding(24.0)
        .gap(12.0)
        .fill(k.c.page)
        .child(setting(&k, "Hotkey", hotkey))
        .child(setting(&k, "Position", position))
        .child(setting(&k, "Input source", input_source))
        .child(setting(&k, "Files shown", file_limit))
        .child(setting(&k, "Backdrop", backdrop))
        .children(updates(s, &k))
        .child(el().grow(1.0))
        .child(
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .child(
                    text(path)
                        .font(k.ty.ui_compact)
                        .color(k.c.text_muted)
                        .truncate()
                        .grow(1.0),
                )
                .child(open_file),
        )
}

/// Whether to install updates, and where the updater is. A build with no update feed (Nix,
/// `cargo run`) shows none of it.
fn updates(s: &Prefs, k: &Kit) -> Vec<El> {
    if !s.updater.available() {
        return Vec::new();
    }
    let auto = Switch::new(s.config.auto_update)
        .label("Install updates automatically")
        .hide_label()
        .disabled(s.read_only)
        .on_toggle(|s: &mut Prefs| s.set(|c| c.auto_update = !c.auto_update))
        .build(k);

    let status = s.updater.status();
    let action = match &status {
        Status::Ready(_) => Button::new("Restart to Update").on_press(|s: &mut Prefs| {
            s.panel.update(|ui, _| ui.restart_to_update());
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
    let version = row()
        .grow(1.0)
        .gap(12.0)
        .center_y()
        .child(
            text(format!("{} · {line}", s.updater.version()))
                .font(k.ty.ui)
                .color(k.c.text_muted)
                .truncate()
                .grow(1.0),
        )
        .child(action);
    vec![setting(k, "Updates", auto), setting(k, "Version", version)]
}

/// The `n` segments of `control`, locked when config.toml can't be written.
fn segments(s: &Prefs, control: SegmentedControl, n: usize) -> SegmentedControl {
    if !s.read_only {
        return control;
    }
    (0..n).fold(control, |c, i| c.segment_state(i, State::Disabled))
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

fn setting(k: &Kit, label: &str, control: El) -> El {
    row()
        .w_full()
        .min_h(36.0)
        .gap(16.0)
        .center_y()
        .child(
            text(label.to_owned())
                .font(k.ty.ui)
                .color(k.c.text_base)
                .w(LABEL_WIDTH),
        )
        .child(control)
}
