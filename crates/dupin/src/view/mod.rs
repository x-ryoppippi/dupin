//! The panel: a query field over a virtualised result list.

mod actions;
mod detail;
mod extensions;
mod files;
mod form;
mod header;
mod notices;
mod picker;
mod results;
mod scripts;
mod theme;

use rune::{Color, Cx, El, SystemMaterial, col, effect};

use crate::ui::Ui;

use theme::divider;

pub use header::HEADER_H;
pub(crate) use results::visible_rows;
pub use results::{list_fit, row_key, wanted_rows};
pub use theme::{ResolvedTheme, theme_preview};

/// How much of the theme's card color covers the glass.
const FROST: f32 = 0.45;

pub fn view(s: &Ui, cx: &mut Cx<Ui>) -> El {
    if s.grows_up() {
        // Rune moves a fitting window's frame on a spring and lays its content out from the
        // top; a panel held at its bottom edge would show the field sliding while both move.
        // Jumping lands the frame and the layout in the same frame.
        cx.reduced_motion(true);
    }
    let dark = cx.system_dark();
    let t = ResolvedTheme::new(&s.theme, dark);
    let content = if let Some(picker) = &s.picker {
        picker::view(s, &picker.values, picker.selected)
    } else {
        launcher(s, &t, dark)
    };
    // Headless and opaque windows draw the same card in place of Liquid Glass.
    col().w_full().child(
        col()
            .key("panel")
            .w_full()
            .radius(s.active_radius)
            .continuous_corners()
            .fill(t.card)
            .system_material(SystemMaterial::Glass)
            .child(content),
    )
}

/// `parts`, listed from the field outwards, in the order they stack from the panel's top.
fn from_field(s: &Ui, mut parts: Vec<El>) -> Vec<El> {
    if s.grows_up() {
        parts.reverse();
    }
    parts
}

fn launcher(s: &Ui, t: &ResolvedTheme, dark: bool) -> El {
    let accent = t.accent;

    // A light frost over the glass keeps text readable over whatever is behind the panel.
    let ink = if dark { Color::WHITE } else { Color::BLACK };
    let mut panel = col()
        .key("frost")
        .w_full()
        .radius(s.active_radius)
        .continuous_corners()
        .clip_rounded(s.active_radius)
        .fill(t.card.alpha(FROST))
        .border(1.0, ink.alpha(0.08));
    if s.backdrop.module().is_some() {
        panel = panel.child(
            effect(crate::backdrop::MARK)
                .key("backdrop")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .effect_data([accent.r, accent.g, accent.b, if dark { 1.0 } else { 0.0 }]),
        );
    }
    panel.children(from_field(s, launcher_parts(s, t, accent)))
}

/// What the panel shows, from the field outwards.
fn launcher_parts(s: &Ui, t: &ResolvedTheme, accent: Color) -> Vec<El> {
    if let Some(entry) = &s.script_entry {
        return scripts::script_arguments(s, entry, t).into();
    }
    let mut parts = vec![if s.action_screen.is_some() {
        actions::action_header(s, t)
    } else {
        header::header(s, t, accent)
    }];
    notices::panel_notices(&mut parts, s, t, accent);
    if let Some(card) = &s.script_card {
        parts.extend([divider(t), scripts::script_inline(card, t)]);
        return parts;
    }
    notices::search_notices(&mut parts, s, t);
    let items = s.session.items();
    if let Some(draft) = s.form_draft() {
        parts.extend([
            divider(t),
            form::form(draft, s.list_budget, s.grows_up(), t, accent),
        ]);
    } else if let Some(renaming) = s.session.renaming() {
        parts.extend([divider(t), files::rename_status(s, renaming, t)]);
    } else if s.action_screen.is_some() {
        parts.extend([divider(t), actions::action_list(s, t, accent)]);
    } else if !items.is_empty() && header::query_problem(s).is_none() {
        parts.extend([divider(t), results::results(s, items, t, accent)]);
    } else if let Some(hint) = header::hint_row(s, t) {
        parts.extend([divider(t), hint]);
    }
    if s.session.destination().is_some() {
        parts.extend([divider(t), files::destination_hint(t)]);
    }
    parts
}
