//! The settings window (⌘, in the panel): the hotkey, where the panel opens, the input source
//! and how many files a search shows. Changes apply to the panel at once and are written back
//! to config.toml. A read-only config.toml (one Nix manages) shows its values, locked.

use std::sync::Arc;

use dupin_core::{Config, InputSource, Position};
use rune::{AppHandle, Cx, El, Shortcut, ShortcutError, Tx, col, el, row, text};
use rune_kit::{
    Button, Kit, SegmentedControl, ShortcutRecorder, ShortcutRecorderState, State, shortcut_error_message,
};

use crate::hotkey::Binding;
use crate::ui::Ui;

pub const SIZE: (f32, f32) = (560.0, 340.0);

const FILE_LIMITS: [usize; 4] = [50, 100, 200, 500];
const LABEL_WIDTH: f32 = 150.0;

pub struct Prefs {
    config: Config,
    recorder: ShortcutRecorderState,
    /// Who holds the chord the last refusal was for, when known.
    owner: Option<String>,
    binding: Arc<Binding>,
    panel: AppHandle<Ui>,
    read_only: bool,
}

impl Prefs {
    pub fn new(config: Config, binding: Arc<Binding>, panel: AppHandle<Ui>) -> Prefs {
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
