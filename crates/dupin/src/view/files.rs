use rune::tok::type_;
use rune::{El, col, el, row, text};

use crate::ui::{ICON_SIZE, Ui};

use super::theme::ResolvedTheme;

/// What is being renamed, and what Enter will do with the name typed so far.
pub(super) fn rename_status(s: &Ui, renaming: &dupin_core::session::Renaming, t: &ResolvedTheme) -> El {
    let icon = match s.icons.get(renaming.path()) {
        Some(image) => el().size(ICON_SIZE, ICON_SIZE).pixels(image.pixels()),
        None => el().size(ICON_SIZE, ICON_SIZE),
    };
    let folder = renaming
        .path()
        .parent()
        .map(|parent| dupin_core::item::tilde(&parent.to_string_lossy()))
        .unwrap_or_default();
    let (status, color) = match renaming.problem() {
        Some(problem) => (problem, t.destructive),
        None => ("↩ Rename · esc Cancel", t.text_secondary),
    };
    row()
        .key("rename")
        .w_full()
        .h(s.theme.row_height as f32)
        .px(18.0)
        .gap(12.0)
        .center_y()
        .child(icon)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(renaming.path().file_name().unwrap_or_default().to_string_lossy())
                        .font(t.font(type_::BODY))
                        .color(t.text)
                        .truncate(),
                )
                .child(
                    text(folder)
                        .font(t.font(type_::LABEL))
                        .color(t.text_secondary)
                        .truncate(),
                ),
        )
        .child(text(status).font(t.font(type_::LABEL)).color(color))
}

pub(super) fn destination_hint(t: &ResolvedTheme) -> El {
    row().w_full().px(20.0).py(8.0).child(
        text("↩ Choose folder · esc Cancel")
            .font(t.font(type_::LABEL))
            .color(t.text_secondary),
    )
}
