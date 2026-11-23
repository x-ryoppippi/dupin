use rune::{El, Icon, PopupMenu, TextStyle, Tx, col, el, icon, row, text};
use rune_kit::{Kit, Select, SelectAppearance};

use super::Prefs;

pub(super) const ROW_PX: f32 = 16.0;

/// The widest a row's value grows before it truncates, leaving its name and description room.
const VALUE_MAX_W: f32 = 200.0;
pub(super) const DESCRIPTION: TextStyle = TextStyle::new(12.0, 440.0).lh(16.0);

/// Where a row's menu opens: its value's trigger, which rune-kit keys after the Select.
pub(super) fn anchor(key: &str) -> String {
    format!("{key}-trigger")
}

/// Open a menu of `options` (id, title) over the row's value, the current one checked, and
/// apply the pick.
pub(super) fn choose(
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

/// A row's current value and a chevron, which `open` turns into a menu of the others.
pub(super) fn menu_value(
    locked: bool,
    k: &Kit,
    key: &str,
    name: &str,
    value: &str,
    open: fn(&mut Prefs, &mut Tx),
) -> El {
    let trigger = Select::new(key, &[value])
        .appearance(SelectAppearance::Inline)
        .label(name)
        .current(Some(value))
        .disabled(locked)
        .trigger(k)
        .max_w(VALUE_MAX_W)
        .mr(-8.0);
    if locked {
        trigger
    } else {
        trigger.on_press_tx(open)
    }
}

/// A row's action: a label and an icon, quiet until hovered.
pub(super) fn value_button(k: &Kit, key: &str, label: &str, glyph: &'static Icon) -> El {
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
pub(super) fn card(k: &Kit, hair: f32, rows: Vec<El>) -> El {
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
pub(super) fn note(k: &Kit, words: &str) -> El {
    text(words.to_owned())
        .font(DESCRIPTION)
        .color(k.c.text_muted)
        .mx(ROW_PX)
        .wrap()
}

/// `$HOME/x` as `~/x`.
pub(super) fn tilde(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => path
            .strip_prefix(&home)
            .map_or_else(|| path.to_owned(), |rest| format!("~{rest}")),
        _ => path.to_owned(),
    }
}

/// A row: an icon, the setting's name and what it does, and its control at the trailing edge.
pub(super) fn setting(k: &Kit, glyph: &'static Icon, label: &str, detail: Option<&str>, control: El) -> El {
    setting_with_lock(k, glyph, label, detail, control, None)
}

pub(super) fn lock_note(k: &Kit, note: &'static str) -> El {
    text(note).font(DESCRIPTION).color(k.c.text_muted)
}

pub(super) fn setting_with_lock(
    k: &Kit,
    glyph: &'static Icon,
    label: &str,
    detail: Option<&str>,
    control: El,
    note: Option<&'static str>,
) -> El {
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
        .child(words.children(note.map(|note| lock_note(k, note))))
        .child(control)
}
