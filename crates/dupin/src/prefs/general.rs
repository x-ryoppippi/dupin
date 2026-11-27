use dupin_core::config::HotkeyTarget;
use dupin_core::{DisplaySelection, InputSource, Position};
use dupin_mac::login_item::{LoginItem, Status as LoginStatus};
use rune::{El, Shortcut, ShortcutError, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant, Kit, ShortcutRecorder, Switch, shortcut_error_message};

use super::Prefs;
use super::controls::{ROW_PX, card, choose, menu_value, setting, setting_with_lock, value_button};
use crate::hotkey::LauncherShortcut;
use crate::icons;

const DISPLAY: &str = "display";
const POSITION: &str = "position";
const INPUT_SOURCE: &str = "input-source";

pub(super) fn page(s: &Prefs, k: &Kit) -> Vec<El> {
    let rows = vec![
        hotkey_setting(s, k),
        selection_hotkey_setting(s, k),
        display_setting(s, k),
        position_setting(s, k),
        input_source_setting(s, k),
        login_setting(s, k),
        automation_setting(s, k),
        telemetry_setting(s, k),
        relaunch_setting(k),
    ];
    let mut body = vec![card(k, rows)];
    if !s.config.bindings.is_empty() {
        body.push(configured_hotkeys(s, k));
    }
    body
}

fn shortcut_recorder(s: &Prefs, k: &Kit, kind: LauncherShortcut, key: &str, label: &str, locked: bool) -> El {
    let i = kind as usize;
    let shortcut_problem = s.shortcut_problems[i].clone();
    let refusal = move |error: &ShortcutError, chord: Option<&Shortcut>| {
        shortcut_problem
            .as_ref()
            .filter(|_| *error == ShortcutError::Conflict)
            .cloned()
            .unwrap_or_else(|| shortcut_error_message(error, chord))
    };
    let recorder = ShortcutRecorder::new(key, &s.recorders[i], move |s: &mut Prefs| &mut s.recorders[i])
        .label(label)
        .width(180.0)
        .on_change(move |s: &mut Prefs, next, _| s.change_hotkey(kind, next))
        .error_message(refusal)
        .disabled(locked)
        .build(k);
    if locked {
        // Rune disables recorder input without applying the kit's usual disabled dimming.
        recorder.layer_opacity(0.5)
    } else {
        recorder
    }
}

/// The panel hotkey's row, on General and in the welcome.
pub(super) fn hotkey_setting(s: &Prefs, k: &Kit) -> El {
    let kind = LauncherShortcut::Panel;
    let locked = s.locked("hotkey");
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
    setting_with_lock(
        k,
        &icons::KEYBOARD,
        "Hotkey",
        Some(&detail),
        shortcut_recorder(s, k, kind, "hotkey", "Show Dupin", locked),
        s.lock_note(locked),
    )
}

fn selection_hotkey_setting(s: &Prefs, k: &Kit) -> El {
    let locked = s.locked("selection_hotkey");
    setting_with_lock(
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
            locked,
        ),
        s.lock_note(locked),
    )
}

fn display_setting(s: &Prefs, k: &Kit) -> El {
    let locked = s.locked("display");
    let display = menu_value(
        locked,
        k,
        DISPLAY,
        "Display",
        display_option(s.config.display).1,
        pick_display,
    );
    setting_with_lock(
        k,
        &icons::PANEL,
        "Display",
        Some(if s.config.display == DisplaySelection::Active {
            "Uses the focused window's display when Accessibility access allows"
        } else {
            "Which display the panel opens on"
        }),
        display,
        s.lock_note(locked),
    )
}

fn pick_display(s: &mut Prefs, tx: &mut Tx) {
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
    choose(
        tx,
        DISPLAY,
        options,
        display_option(s.config.display).0,
        |s, id| {
            if let Some(value) = CHOICES.into_iter().find(|value| display_option(*value).0 == id) {
                s.set(|config| config.display = value);
            }
        },
    );
}

fn display_option(display: DisplaySelection) -> (&'static str, &'static str) {
    match display {
        DisplaySelection::Mouse => ("mouse", "Mouse pointer"),
        DisplaySelection::Main => ("main", "Main display"),
        DisplaySelection::Active => ("active", "Focused window"),
    }
}

fn position_setting(s: &Prefs, k: &Kit) -> El {
    let locked = s.locked("position");
    let position = menu_value(
        locked,
        k,
        POSITION,
        "Position",
        position_title(s.config.position),
        pick_position,
    );
    setting_with_lock(
        k,
        &icons::PANEL,
        "Position",
        Some("Where on the display the panel opens"),
        position,
        s.lock_note(locked),
    )
}

fn pick_position(s: &mut Prefs, tx: &mut Tx) {
    const POSITIONS: [Position; 3] = [Position::Center, Position::Top, Position::Bottom];
    let options = POSITIONS
        .map(|p| (position_id(p).to_owned(), position_title(p).to_owned()))
        .to_vec();
    choose(tx, POSITION, options, position_id(s.config.position), |s, id| {
        if let Some(position) = POSITIONS.into_iter().find(|p| position_id(*p) == id) {
            s.set(|c| c.position = position);
        }
    });
}

/// The position's name in config.toml.
fn position_id(position: Position) -> &'static str {
    match position {
        Position::Center => "center",
        Position::Top => "top",
        Position::Bottom => "bottom",
    }
}

fn position_title(position: Position) -> &'static str {
    match position {
        Position::Center => "Center",
        Position::Top => "Top",
        Position::Bottom => "Bottom",
    }
}

fn input_source_setting(s: &Prefs, k: &Kit) -> El {
    let locked = s.locked("input_source");
    let input_source = menu_value(
        locked,
        k,
        INPUT_SOURCE,
        "Input source",
        &input_source_title(&s.config.input_source),
        pick_input_source,
    );
    setting_with_lock(
        k,
        &icons::LANGUAGES,
        "Input source",
        Some("ASCII switches an input method to Roman while the panel shows"),
        input_source,
        s.lock_note(locked),
    )
}

fn pick_input_source(s: &mut Prefs, tx: &mut Tx) {
    let current = &s.config.input_source;
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
    choose(tx, INPUT_SOURCE, options, selected, |s, id| match id {
        "ascii" => s.set(|c| c.input_source = InputSource::Ascii),
        "keep" => s.set(|c| c.input_source = InputSource::Keep),
        _ => {}
    });
}

fn input_source_title(source: &InputSource) -> String {
    match source {
        InputSource::Ascii => "ASCII".to_owned(),
        InputSource::Keep => "Keep".to_owned(),
        InputSource::Id(id) => id.rsplit('.').next().unwrap_or(id).to_owned(),
    }
}

/// Open the shaders folder in Finder, for a user to drop a file in. Settings makes it first
pub(super) struct LoginPreference {
    item: LoginItem,
    status: LoginStatus,
    error: Option<String>,
}

impl LoginPreference {
    pub(super) fn current() -> Option<Self> {
        objc2::MainThreadMarker::new()
            .and_then(LoginItem::for_current_app)
            .map(|item| Self {
                status: item.status(),
                item,
                error: None,
            })
    }
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
    pub fn refresh_login_item(&mut self) {
        if let Some(login) = &mut self.login {
            login.status = login.item.status();
            login.error = None;
        }
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
}

pub(super) fn login_setting(s: &Prefs, k: &Kit) -> El {
    let login = s.login.as_ref();
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

fn automation_setting(s: &Prefs, k: &Kit) -> El {
    let locked = s.locked("system_automation");
    let automation_enabled = s.config.system_automation;
    let automation = Switch::new(automation_enabled)
        .label("System Command Automation")
        .hide_label()
        .disabled(locked)
        .on_toggle(move |s: &mut Prefs| s.set(|c| c.system_automation = !automation_enabled))
        .build(k);
    setting_with_lock(
        k,
        &rune_theme_opencode::icons::SHIELD,
        "System Command Automation",
        Some("Show system commands that may ask for macOS Automation when confirmed"),
        automation,
        s.lock_note(locked),
    )
}

/// The usage ping's row, on General and in the welcome: what it sends, a link to the details and
/// the switch that turns it off.
pub(super) fn telemetry_setting(s: &Prefs, k: &Kit) -> El {
    use crate::telemetry::{DOCS_URL, Off, SENT};
    let locked = s.locked("telemetry");
    let enabled = s.config.telemetry;
    let off = crate::telemetry::off(enabled);
    let by_env = matches!(off, Some(Off::Env(_)));
    let switch = Switch::new(enabled && !by_env)
        .label("Anonymous usage ping")
        .hide_label()
        .disabled(locked || by_env)
        .on_toggle(move |s: &mut Prefs| s.set(|c| c.telemetry = !enabled))
        .build(k)
        .key("telemetry");
    let details = Button::new("Details")
        .variant(ButtonVariant::Ghost)
        .on_press(|_: &mut Prefs| {
            std::thread::spawn(|| {
                _ = dupin_mac::workspace::run(&dupin_mac::workspace::Action::OpenUrl(DOCS_URL.into()));
            });
        })
        .build(k)
        .key("telemetry-details");
    let detail = match off {
        Some(off @ (Off::Env(_) | Off::Unofficial | Off::Unsigned)) => format!("{SENT} {}.", off.describe()),
        Some(Off::Setting) | None => SENT.to_owned(),
    };
    setting_with_lock(
        k,
        &icons::INFO,
        "Anonymous usage ping",
        Some(&detail),
        row().gap(8.0).center_y().child(details).child(switch),
        s.lock_note(locked),
    )
}

fn relaunch_setting(k: &Kit) -> El {
    let relaunch = value_button(k, "relaunch", "Relaunch", &icons::REFRESH).on_press(|s: &mut Prefs| {
        s.panel.update(|ui, tx| ui.relaunch(tx));
    });
    setting(
        k,
        &icons::REFRESH,
        "Relaunch Dupin",
        Some("Restart, reload configuration and cancel running scripts"),
        relaunch,
    )
}

fn configured_hotkeys(s: &Prefs, k: &Kit) -> El {
    let rows = s
        .config
        .bindings
        .iter()
        .map(|(chord, target)| {
            let keys = chord
                .parse::<Shortcut>()
                .map_or_else(|_| chord.clone(), |shortcut| shortcut.keycaps().join(""));
            let (kind, name) = match target {
                HotkeyTarget::App { bundle_id, .. } => ("App", bundle_id.clone()),
                HotkeyTarget::Command { command } => ("Command", command.clone()),
                HotkeyTarget::Script { script, .. } => ("Script", script.clone()),
                HotkeyTarget::Dupin { command, query } => (
                    "Dupin command",
                    match query {
                        Some(query) => format!("{command} · {query}"),
                        None => command.to_string(),
                    },
                ),
            };
            setting_with_lock(
                k,
                &icons::KEYBOARD,
                kind,
                Some(&name),
                text(keys).font(k.ty.ui).color(k.c.text_base),
                s.lock_note(s.locked("bindings")),
            )
            .key(chord)
        })
        .collect();

    col()
        .key("configured-hotkeys")
        .gap(4.0)
        .child(
            text("Configured hotkeys")
                .mx(ROW_PX)
                .font(k.ty.body_medium)
                .color(k.c.text_base),
        )
        .child(card(k, rows))
}
