use dupin_mac::window::DisplayNotice;
use rune::tok::type_;
use rune::{Color, El, col, row, text};

use crate::ui::Ui;

use super::theme::{ResolvedTheme, divider};

const WARNING: Color = Color {
    r: 0.95,
    g: 0.55,
    b: 0.15,
    a: 0.14,
};

pub(super) fn panel_notices(mut panel: El, s: &Ui, t: &ResolvedTheme, accent: Color) -> El {
    let notices = [
        ("config-problem", s.config_problem.as_deref()),
        ("hotkey-problem", s.hotkey_problem.as_deref()),
        ("notice", s.notice()),
        ("registration-notice", s.registration_notice.as_deref()),
        ("backdrop-notice", s.backdrop.problem()),
    ];
    for (key, notice) in notices.into_iter().filter_map(|(k, n)| Some((k, n?))) {
        panel = panel.child(divider(t)).child(
            row().key(key).w_full().px(18.0).py(12.0).fill(WARNING).child(
                text(notice)
                    .font(t.font(type_::BODY.lh(18.0)))
                    .color(t.text)
                    .wrap()
                    .w_full(),
            ),
        );
    }
    if let Some(notice) = s.display_notice {
        let message = match notice {
            DisplayNotice::AccessibilityRequired => {
                "Dupin needs Accessibility access to find the focused window. Using the main display."
            }
            DisplayNotice::FocusedWindowUnavailable => {
                "Couldn't find the focused window's display. Using the main display."
            }
        };
        panel = panel.child(divider(t)).child(
            col()
                .key("display-notice")
                .w_full()
                .px(18.0)
                .py(12.0)
                .gap(8.0)
                .fill(WARNING)
                .child(
                    text(message)
                        .font(t.font(type_::BODY.lh(18.0)))
                        .color(t.text)
                        .wrap()
                        .w_full(),
                )
                .children((notice == DisplayNotice::AccessibilityRequired).then(|| {
                    text("Set Up Accessibility")
                        .font(t.font(type_::BODY))
                        .color(accent)
                        .key("open-accessibility-settings")
                        .role("button")
                        .focusable(true)
                        .on_press_tx(Ui::open_accessibility_settings)
                })),
        );
    }
    if s.selection_permission_required {
        panel = panel.child(divider(t)).child(
            col()
                .key("selection-permission")
                .w_full()
                .px(18.0)
                .py(12.0)
                .gap(8.0)
                .fill(WARNING)
                .child(
                    text("Dupin needs Accessibility access to read selected text from other apps.")
                        .font(t.font(type_::BODY.lh(18.0)))
                        .color(t.text)
                        .wrap()
                        .w_full(),
                )
                .children(
                    (s.display_notice != Some(DisplayNotice::AccessibilityRequired)).then(|| {
                        text("Open System Settings")
                            .key("selection-settings")
                            .role("button")
                            .focusable(true)
                            .font(t.font(type_::TITLE))
                            .color(accent)
                            .on_press_tx(Ui::open_accessibility_settings)
                    }),
                ),
        );
    }
    panel
}

pub(super) fn search_notices(mut panel: El, s: &Ui, t: &ResolvedTheme) -> El {
    for (origin, failure) in s.session.failures() {
        panel = panel.child(
            row()
                .key(format!("failure-{origin}"))
                .w_full()
                .px(18.0)
                .py(8.0)
                .child(
                    text(format!(
                        "{}: {failure}",
                        s.session
                            .provider_spec(origin)
                            .map(|spec| spec
                                .presentation
                                .chip
                                .as_deref()
                                .unwrap_or(&spec.presentation.placeholder))
                            .unwrap_or("Search")
                    ))
                    .font(t.font(type_::BODY))
                    .color(t.text_secondary)
                    .wrap()
                    .w_full(),
                ),
        );
    }
    panel
}
