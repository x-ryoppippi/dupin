//! The first-run welcome (#13), shown inside Settings: choosing a hotkey and launching at
//! login, then a short tour of the modes. A marker file beside the history keeps it to one
//! appearance; Show Welcome in the sidebar brings it back.

use std::path::Path;

use rune::{El, el, row, text};
use rune_kit::{Button, ButtonVariant, Keybind, Kit};

use super::controls::{DESCRIPTION, ROW_PX, card, note, setting, value_button};
use super::general::{hotkey_setting, login_setting};
use super::{GUTTER, Page, Prefs, permissions};
use crate::icons;

const MARKER: &str = "welcomed";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Step {
    Hotkey,
    Tour,
}

impl Step {
    pub(super) fn title(self) -> &'static str {
        match self {
            Step::Hotkey => "Welcome to Dupin",
            Step::Tour => "What Dupin finds",
        }
    }
}

/// Whether this launch should welcome its user, recording that it has. Someone with a history
/// has used Dupin before the welcome existed, and a headless run has no one to welcome.
pub fn first_run() -> bool {
    if rune::tooling::is_headless() {
        return false;
    }
    let Some(dir) = crate::store::support_dir() else {
        return false;
    };
    if !due(&dir) {
        return false;
    }
    if let Err(e) = mark(&dir) {
        crate::log::warn(&format!("couldn't record the welcome as shown: {e}"));
    }
    true
}

fn due(dir: &Path) -> bool {
    !dir.join(MARKER).exists() && !dir.join(crate::store::FILE).exists()
}

fn mark(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(MARKER), "")
}

pub(super) fn page(s: &Prefs, k: &Kit, hair: f32, step: Step) -> Vec<El> {
    match step {
        Step::Hotkey => hotkey_step(s, k, hair),
        Step::Tour => tour_step(s, k, hair),
    }
}

fn intro(k: &Kit, words: &str) -> El {
    text(words.to_owned())
        .font(DESCRIPTION)
        .color(k.c.text_base)
        .mx(ROW_PX)
        .wrap()
}

fn hotkey_step(s: &Prefs, k: &Kit, hair: f32) -> Vec<El> {
    let mut rows = vec![hotkey_setting(s, k), login_setting(s, k)];
    if permissions::available() {
        let review = value_button(k, "welcome-permissions", "Review", &icons::SHIELD)
            .on_press(|s: &mut Prefs| s.show(Page::Permissions));
        rows.push(setting(
            k,
            &icons::SHIELD,
            "Permissions",
            Some("Review access or request it before using a feature that needs it"),
            review,
        ));
    }
    vec![
        intro(
            k,
            "Dupin lives in the background and opens from a hotkey in any app. Pick one you can \
             spare; Settings can change it later.",
        ),
        card(k, hair, rows),
        note(
            k,
            "⌘Space is Spotlight's shortcut too. To give it to Dupin, turn off Show Spotlight search in \
             System Settings → Keyboard → Keyboard Shortcuts → Spotlight, or choose another hotkey. \
             Alfred, Raycast and input source switching (⌃Space) often hold similar keys; when one \
             does, Dupin says which app has it.",
        ),
    ]
}

fn tour_step(s: &Prefs, k: &Kit, hair: f32) -> Vec<El> {
    let mode = |glyph, name: &str, detail: &str, keys: &[&str]| {
        setting(k, glyph, name, Some(detail), Keybind::new(keys).build(k))
    };
    let mut rows = vec![
        mode(
            &icons::SEARCH,
            "Apps",
            "Type an app's name, as Finder shows it or by Spotlight's keywords",
            &["abc"],
        ),
        mode(
            &icons::FILES,
            "Files",
            "Space on an empty query searches file names with Spotlight",
            &["Space"],
        ),
        mode(
            &icons::FOLDER,
            "Inside files",
            "in, then Space, searches the text Spotlight indexed from inside files",
            &["in", "Space"],
        ),
    ];
    if let Some(first) = s.config.dictionary_keywords.first() {
        rows.push(mode(
            &icons::LANGUAGES,
            "Dictionary",
            &format!(
                "{}, then Space, looks a word up in macOS dictionaries",
                or_list(&s.config.dictionary_keywords)
            ),
            &[first, "Space"],
        ));
    }
    rows.push(mode(
        &icons::SLIDERS,
        "Actions",
        "→ or ⌘K shows what you can do with the selected result",
        &["⌘", "K"],
    ));
    vec![
        intro(
            k,
            "Everything starts from the one field. A few keys switch what it searches.",
        ),
        card(k, hair, rows),
        note(
            k,
            "⌘, in the panel opens Settings, where Show Welcome brings this guide back.",
        ),
    ]
}

fn or_list(words: &[String]) -> String {
    match words {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

/// Skip, the step count, Back and Continue (Done on the last step).
pub(super) fn footer(k: &Kit, step: Step) -> El {
    let finish = |s: &mut Prefs| s.show(Page::General);
    let skip = Button::new("Skip")
        .variant(ButtonVariant::Ghost)
        .on_press(finish)
        .build(k)
        .key("welcome-skip");
    let count = text(match step {
        Step::Hotkey => "Step 1 of 2",
        Step::Tour => "Step 2 of 2",
    })
    .font(k.ty.small)
    .color(k.c.text_muted);
    let mut bar = row()
        .key("welcome-footer")
        .w_full()
        .h(56.0)
        .px(GUTTER)
        .gap(8.0)
        .center_y()
        .children((step == Step::Hotkey).then_some(skip))
        .child(el().grow(1.0))
        .child(count.mr(8.0));
    bar = match step {
        Step::Hotkey => bar.child(
            Button::new("Continue")
                .variant(ButtonVariant::Submit)
                .on_press(|s: &mut Prefs| s.welcome = Some(Step::Tour))
                .build(k)
                .key("welcome-continue"),
        ),
        Step::Tour => bar
            .child(
                Button::new("Back")
                    .variant(ButtonVariant::Outline)
                    .on_press(|s: &mut Prefs| s.welcome = Some(Step::Hotkey))
                    .build(k)
                    .key("welcome-back"),
            )
            .child(
                Button::new("Done")
                    .variant(ButtonVariant::Submit)
                    .on_press(finish)
                    .build(k)
                    .key("welcome-done"),
            ),
    };
    bar
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn welcomes_only_a_user_without_history_once() {
        let dir = std::env::temp_dir().join(format!("dupin-welcome-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        assert!(due(&dir), "a missing directory is a first run");
        mark(&dir).unwrap();
        assert!(!due(&dir), "the marker keeps it to one appearance");
        std::fs::remove_file(dir.join(MARKER)).unwrap();
        std::fs::write(dir.join(crate::store::FILE), "{}").unwrap();
        assert!(!due(&dir), "a history means Dupin was used before");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn keywords_read_as_alternatives() {
        let words = |w: &[&str]| w.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
        assert_eq!(or_list(&words(&["define"])), "define");
        assert_eq!(or_list(&words(&["define", "d"])), "define or d");
        assert_eq!(or_list(&words(&["define", "d", "word"])), "define, d or word");
    }
}
