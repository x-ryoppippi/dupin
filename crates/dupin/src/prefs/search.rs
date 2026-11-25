use dupin_core::config::Retention;
use rune::{El, Tx, col, text};
use rune_kit::{Kit, Switch};

use super::Prefs;
use super::controls::{
    DESCRIPTION, ROW_PX, card, choose, menu_value, setting, setting_with_lock, value_button,
};
use crate::icons;

const FILE_LIMITS: [usize; 4] = [50, 100, 200, 500];
const FILE_LIMIT: &str = "file-limit";
const CLIPBOARD_RETENTION: &str = "clipboard-retention";

pub(super) fn page(s: &Prefs, k: &Kit, hair: f32) -> Vec<El> {
    let mut rows = vec![
        calculator_setting(s, k),
        currency_setting(s, k),
        recent_setting(s, k),
        file_limit_setting(s, k),
        calendar_setting(s, k),
        clipboard_setting(s, k),
        clipboard_retention_setting(s, k),
        clipboard_images_setting(s, k),
        clipboard_files_setting(s, k),
        snippet_paste_setting(s, k),
        window_management_setting(k),
    ];
    rows.extend(snippet_accessibility(s, k));
    let mut body = vec![card(k, hair, rows)];
    body.extend(
        s.scripts_notice
            .as_deref()
            .map(|notice| scripts_notice(k, notice)),
    );
    let composed = crate::composition::summary(&s.config.advanced);
    if !composed.is_empty() {
        body.push(advanced_composition(k, &composed));
    }
    body
}

/// What `[advanced]` composes, shown and not edited here: it is written in config.toml.
fn advanced_composition(k: &Kit, lines: &[String]) -> El {
    let mut block = col().key("advanced-composition").mx(ROW_PX).gap(4.0).child(
        text("Advanced composition")
            .font(k.ty.body_medium)
            .color(k.c.text_base),
    );
    for line in lines {
        block = block.child(text(line).w_full().font(DESCRIPTION).color(k.c.text_muted).wrap());
    }
    block.child(
        text("Edit [advanced] in config.toml to change it.")
            .font(DESCRIPTION)
            .color(k.c.text_muted),
    )
}

fn calculator_setting(s: &Prefs, k: &Kit) -> El {
    let locked = s.locked("calculator");
    let next_calculator = !s.config.calculator;
    let calculator = Switch::new(s.config.calculator)
        .label("Calculator")
        .hide_label()
        .disabled(locked)
        .on_toggle(move |s: &mut Prefs| s.set(|c| c.calculator = next_calculator))
        .build(k);
    setting_with_lock(
        k,
        &icons::CALCULATOR,
        "Calculator",
        Some("Show calculations and unit conversions in app search"),
        calculator,
        s.lock_note(locked),
    )
}

fn currency_setting(s: &Prefs, k: &Kit) -> El {
    let locked = s.locked("calculator_currency");
    let next_currency = !s.config.calculator_currency;
    let currency = Switch::new(s.config.calculator_currency)
        .label("Currency conversion")
        .hide_label()
        .disabled(locked || !s.config.calculator)
        .on_toggle(move |s: &mut Prefs| s.set(|c| c.calculator_currency = next_currency))
        .build(k);
    setting_with_lock(
        k,
        &icons::CALCULATOR,
        "Currency conversion",
        Some("Fetch European Central Bank rates in the background"),
        currency,
        s.lock_note(locked),
    )
}

fn recent_setting(s: &Prefs, k: &Kit) -> El {
    let locked = s.locked("show_recent");
    let show_recent = !s.config.show_recent;
    let recent = Switch::new(s.config.show_recent)
        .label("Show recent items")
        .hide_label()
        .disabled(locked)
        .on_toggle(move |s: &mut Prefs| s.set(|c| c.show_recent = show_recent))
        .build(k);
    setting_with_lock(
        k,
        &icons::PANEL,
        "Show recent items",
        Some("Show recently and frequently used apps and files when the search is empty"),
        recent,
        s.lock_note(locked),
    )
}

fn file_limit_setting(s: &Prefs, k: &Kit) -> El {
    let locked = s.locked("file_limit");
    let limit = menu_value(
        locked,
        k,
        FILE_LIMIT,
        "Files shown",
        &s.config.file_limit.to_string(),
        pick_file_limit,
    );
    setting_with_lock(
        k,
        &icons::FILES,
        "Files shown",
        Some("The most files a Spotlight search lists"),
        limit,
        s.lock_note(locked),
    )
}

fn pick_file_limit(s: &mut Prefs, tx: &mut Tx) {
    let options = FILE_LIMITS
        .iter()
        .map(|n| (n.to_string(), n.to_string()))
        .collect();
    let selected = s.config.file_limit.to_string();
    choose(tx, FILE_LIMIT, options, &selected, |s, id| {
        if let Ok(n) = id.parse() {
            s.set(|c| c.file_limit = n);
        }
    });
}

fn calendar_setting(s: &Prefs, k: &Kit) -> El {
    let locked = s.locked("calendar");
    let calendar_enabled = !s.config.calendar.enabled;
    let calendar = Switch::new(s.config.calendar.enabled)
        .label("Calendar search")
        .hide_label()
        .disabled(locked)
        .on_toggle(move |s: &mut Prefs| s.set(|c| c.calendar.enabled = calendar_enabled))
        .build(k);
    setting_with_lock(
        k,
        &icons::SEARCH,
        "Calendar search",
        Some("Type cal then Space to search events. Requires Calendar access."),
        calendar,
        s.lock_note(locked),
    )
}

fn clipboard_setting(s: &Prefs, k: &Kit) -> El {
    let locked = s.locked("clipboard");
    let next_clipboard = !s.config.clipboard.enabled;
    let clipboard = Switch::new(s.config.clipboard.enabled)
        .label("Clipboard history")
        .hide_label()
        .disabled(locked)
        .on_toggle(move |s: &mut Prefs| s.set(|c| c.clipboard.enabled = next_clipboard))
        .build(k);
    setting_with_lock(
        k,
        &icons::CLIPBOARD,
        "Clipboard history",
        Some("Keeps what you copy, encrypted on this Mac; type clip and Space to search it"),
        clipboard,
        s.lock_note(locked),
    )
}

fn clipboard_retention_setting(s: &Prefs, k: &Kit) -> El {
    let locked = s.locked("clipboard");
    let retention = menu_value(
        locked || !s.config.clipboard.enabled,
        k,
        CLIPBOARD_RETENTION,
        "Keep copies for",
        s.config.clipboard.retention.label(),
        pick_clipboard_retention,
    );
    setting_with_lock(
        k,
        &icons::CLOCK,
        "Keep copies for",
        Some("Older entries are deleted from the history"),
        retention,
        s.lock_note(locked),
    )
}

fn pick_clipboard_retention(s: &mut Prefs, tx: &mut Tx) {
    let options = Retention::ALL
        .iter()
        .map(|retention| (retention.label().to_owned(), retention.label().to_owned()))
        .collect();
    let selected = s.config.clipboard.retention.label();
    choose(tx, CLIPBOARD_RETENTION, options, selected, |s, id| {
        if let Some(retention) = Retention::ALL
            .into_iter()
            .find(|retention| retention.label() == id)
        {
            s.set(|c| c.clipboard.retention = retention);
        }
    });
}

fn clipboard_images_setting(s: &Prefs, k: &Kit) -> El {
    let locked = s.locked("clipboard");
    let next = !s.config.clipboard.include_images;
    let images = Switch::new(s.config.clipboard.include_images)
        .label("Clipboard images")
        .hide_label()
        .disabled(locked || !s.config.clipboard.enabled)
        .on_toggle(move |s: &mut Prefs| s.set(|c| c.clipboard.include_images = next))
        .build(k);
    setting_with_lock(
        k,
        &icons::IMAGE,
        "Clipboard images",
        Some("Keep copied images and screenshots, converted to PNG"),
        images,
        s.lock_note(locked),
    )
}

fn clipboard_files_setting(s: &Prefs, k: &Kit) -> El {
    let locked = s.locked("clipboard");
    let next = !s.config.clipboard.include_files;
    let files = Switch::new(s.config.clipboard.include_files)
        .label("Clipboard files")
        .hide_label()
        .disabled(locked || !s.config.clipboard.enabled)
        .on_toggle(move |s: &mut Prefs| s.set(|c| c.clipboard.include_files = next))
        .build(k);
    setting_with_lock(
        k,
        &icons::FILES,
        "Clipboard files",
        Some("Keep files copied in Finder, by their location rather than a copy"),
        files,
        s.lock_note(locked),
    )
}

fn snippet_paste_setting(s: &Prefs, k: &Kit) -> El {
    let locked = s.locked("snippet_paste");
    let next_paste = !s.config.snippet_paste;
    let paste = Switch::new(s.config.snippet_paste)
        .label("Paste snippets")
        .hide_label()
        .disabled(locked)
        .on_toggle(move |s: &mut Prefs| s.set(|c| c.snippet_paste = next_paste))
        .build(k);
    setting_with_lock(
        k,
        &icons::KEYBOARD,
        "Paste snippets",
        Some("Enter pastes a snippet into the front app. Requires Accessibility access."),
        paste,
        s.lock_note(locked),
    )
}

fn window_management_setting(k: &Kit) -> El {
    let open = value_button(
        k,
        "open-window-management",
        "Extensions",
        &rune_theme_opencode::icons::CHEVRON_RIGHT,
    )
    .on_press(|s: &mut Prefs| s.show_extensions());
    setting(
        k,
        &icons::PANEL,
        "Window Management",
        Some("Now an extension: turn on Window Management in Settings → Extensions"),
        open,
    )
}

fn snippet_accessibility(s: &Prefs, k: &Kit) -> Option<El> {
    if !s.config.snippet_paste || dupin_mac::paste::allowed() {
        return None;
    }
    let open = value_button(
        k,
        "open-accessibility",
        "Open",
        &rune_theme_opencode::icons::CHEVRON_RIGHT,
    )
    .on_press(|_: &mut Prefs| {
        std::thread::spawn(|| {
            let action = dupin_mac::workspace::Action::OpenUrl(dupin_mac::accessibility::SETTINGS_URL.into());
            if let Err(error) = dupin_mac::workspace::run(&action) {
                crate::log::warn(&error);
            }
        });
    });
    Some(setting(
        k,
        &icons::INFO,
        "Accessibility access",
        Some("Pasting needs Dupin allowed in Privacy & Security › Accessibility"),
        open,
    ))
}

fn scripts_notice(k: &Kit, notice: &str) -> El {
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
}
