use std::hash::{DefaultHasher, Hash, Hasher};

use dupin_core::{Item, View};
use rune::tok::type_;
use rune::{Align, Color, El, GrowthSpec, ScrollSpec, Span, TextStyle, col, el, rich, row, text};

use crate::ui::{ICON_SIZE, Ui};

use super::theme::ResolvedTheme;

/// Rows shown before the list scrolls.
const VISIBLE_ROWS: usize = 8;
pub(super) const LIST_HEIGHT_BUDGET: f32 = 416.0;

pub(super) fn visible_rows(row_height: f32) -> usize {
    VISIBLE_ROWS
        .min((LIST_HEIGHT_BUDGET / row_height).floor() as usize)
        .max(1)
}
pub(super) const LIST_PAD: f32 = 6.0;

pub(super) fn results(s: &Ui, items: &[Item], t: &ResolvedTheme, accent: Color) -> El {
    if !items.iter().all(|item| matches!(item.view, View::Text { .. })) {
        let rows = items
            .iter()
            .enumerate()
            .map(|(index, item)| match &item.view {
                View::Text { .. } => result_row(s, item, index, index == s.session.selected(), t, accent),
                View::Detail { .. } | View::Pair { .. } => {
                    content_card(s, item, index, index == s.session.selected(), t, accent)
                }
            })
            .collect::<Vec<_>>();
        let content = col().key("content-rows").w_full().children(rows);
        return col().key("results").w_full().py(LIST_PAD).child(
            col()
                .key("content-scroll")
                .w_full()
                .shrink()
                .max_h(440.0)
                .scroll(ScrollSpec { stick_bottom: false })
                .child(content),
        );
    }
    let shown = items.len().min(visible_rows(s.theme.row_height as f32)) as f32;
    let selected = s.session.selected();
    let rows: Vec<El> = items
        .iter()
        .enumerate()
        .map(|(i, item)| result_row(s, item, i, i == selected, t, accent))
        .collect();
    col().key("results").w_full().py(LIST_PAD).child(
        col()
            .key("scroll")
            .w_full()
            .h(shown * s.theme.row_height as f32)
            .scroll(ScrollSpec { stick_bottom: false })
            .instant_changes()
            .child(
                col()
                    .key("rows")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtualize(s.theme.row_height as f32)
                    .children(rows),
            ),
    )
}

fn content_card(s: &Ui, item: &Item, index: usize, on: bool, t: &ResolvedTheme, accent: Color) -> El {
    let id = s.session.result_key(index);
    let mut card = col()
        .w_full()
        .px(22.0)
        .py(18.0)
        .gap(16.0)
        .radius(8.0)
        .fill(t.card)
        .border(1.0, if on { accent } else { Color::TRANSPARENT });
    let mut header = row().w_full().center_y().gap(12.0);
    let has_title = if let View::Detail { title, .. } = &item.view {
        header = header.child(
            text(title)
                .font(t.font(TextStyle::new(24.0, 600.0)))
                .color(t.text)
                .grow(1.0)
                .min_w(0.0)
                .wrap(),
        );
        true
    } else {
        header = header.child(el().grow(1.0));
        false
    };
    if item.has_actions() {
        header = header.child(
            text("→ Actions")
                .font(t.font(type_::BODY))
                .color(accent)
                .cursor(rune::Cursor::Pointer)
                .hover(t.hover)
                .on_press_tx(move |s: &mut Ui, tx| s.actions(&id, tx)),
        );
    }
    if has_title || item.has_actions() {
        card = card.child(header);
    }
    let body = match &item.view {
        View::Detail { body, .. } => col()
            .key(format!("{}-body", row_key(&s.session.layout_key(index))))
            .w_full()
            .shrink()
            .min_h(20.0)
            .max_h(340.0)
            .scroll(ScrollSpec { stick_bottom: false })
            .child(detail_body(&s.session.layout_key(index), body, t)),
        View::Pair { source, result } => row()
            .key(format!("{}-pair", row_key(&s.session.layout_key(index))))
            .w_full()
            .gap(16.0)
            .center_y()
            .selection_group(true)
            .child(pair_value(
                &s.session.layout_key(index),
                "source",
                source,
                "Source",
                t,
            ))
            .child(
                text("→")
                    .font(t.font(TextStyle::new(28.0, 400.0)))
                    .color(t.text_secondary),
            )
            .child(pair_value(
                &s.session.layout_key(index),
                "result",
                result,
                "Result",
                t,
            )),
        View::Text { .. } => unreachable!("text row"),
    };
    col()
        .key(row_key(&s.session.layout_key(index)))
        .w_full()
        .px(LIST_PAD)
        .py(LIST_PAD)
        .child(card.child(body))
}

fn pair_value(id: &str, part: &str, value: &str, label: &str, t: &ResolvedTheme) -> El {
    col()
        .grow(1.0)
        .min_w(0.0)
        .gap(6.0)
        .child(
            text(value)
                .key(format!("{}-{part}", row_key(id)))
                .w_full()
                .font(t.font(TextStyle::new(28.0, 600.0).lh(36.0)))
                .color(t.text)
                .align(Align::Center)
                .wrap()
                .selectable(true),
        )
        .child(
            text(label)
                .w_full()
                .font(t.font(type_::LABEL))
                .color(t.text_secondary)
                .align(Align::Center),
        )
}

fn detail_body(id: &str, body: &str, t: &ResolvedTheme) -> El {
    let boundaries: Vec<usize> = std::iter::once(0)
        .chain(
            body.match_indices('•')
                .map(|(index, _)| index)
                .filter(|&index| index > 0),
        )
        .chain(std::iter::once(body.len()))
        .collect();
    col()
        .key(format!("{}-selection", row_key(id)))
        .w_full()
        .gap(12.0)
        .selection_group(true)
        .children(boundaries.windows(2).enumerate().map(|(index, range)| {
            text(body[range[0]..range[1]].trim_end())
                .key(format!("{}-paragraph-{index}", row_key(id)))
                .w_full()
                .font(t.font(TextStyle::new(16.0, 400.0).lh(25.0)))
                .color(t.text)
                .wrap()
                .selectable(true)
        }))
}

fn result_row(s: &Ui, item: &Item, index: usize, on: bool, t: &ResolvedTheme, accent: Color) -> El {
    let View::Text { subtitle, .. } = &item.view else {
        unreachable!("text row")
    };
    let icon = item_icon(s, item.icon.as_deref());
    let id = s.session.result_key(index);
    text_row(
        t,
        accent,
        s.theme.row_height as f32,
        on,
        icon,
        title(
            item,
            if on { t.selected_text } else { t.text },
            if on { t.selected_text } else { accent },
            t,
        ),
        subtitle,
    )
    .key(row_key(&s.session.layout_key(index)))
    .on_press_tx(move |s: &mut Ui, tx| s.open(&id, tx))
}

pub(super) fn item_icon(s: &Ui, path: Option<&std::path::Path>) -> El {
    let icon = el().size(ICON_SIZE, ICON_SIZE);
    match path.and_then(|path| s.icons.get(path)) {
        Some(image) => icon.pixels(image.pixels()),
        None => icon,
    }
}

pub(super) fn text_row(
    t: &ResolvedTheme,
    accent: Color,
    height: f32,
    on: bool,
    icon: El,
    title: El,
    subtitle: &str,
) -> El {
    row().w_full().h(height).px(LIST_PAD).child(
        row()
            .w_full()
            .h_full()
            .px(10.0)
            .gap(12.0)
            .radius(8.0)
            .center_y()
            .fill(if on { accent } else { Color::TRANSPARENT })
            .hover(if on { accent } else { t.hover })
            .child(icon)
            .child(
                col().grow(1.0).min_w(0.0).gap(2.0).child(title).child(
                    text(subtitle)
                        .font(t.font(type_::LABEL))
                        .color(if on {
                            t.selected_text.alpha(0.78)
                        } else {
                            t.text_secondary
                        })
                        .truncate(),
                ),
            ),
    )
}

/// The title with the characters the query matched in bold.
fn title(item: &Item, ink: Color, highlight: Color, t: &ResolvedTheme) -> El {
    let byte_of: Vec<usize> = item
        .display_title()
        .char_indices()
        .map(|(b, _)| b)
        .chain([item.display_title().len()])
        .collect();
    let spans = item
        .matched
        .iter()
        .filter(|&&c| c + 1 < byte_of.len())
        .map(|&c| Span::new(byte_of[c], byte_of[c + 1]).bold().color(highlight))
        .collect();
    rich(item.display_title(), spans)
        .font(t.font(TextStyle::new(15.0, 500.0).lh(20.0)))
        .color(ink)
        .truncate()
}

/// Element keys for result rows. Ids are paths, whose slashes would read as key paths in
/// scripts and `scroll_into_view`, so rows are keyed by a hash of the id instead.
pub fn row_key(id: &str) -> String {
    let mut h = DefaultHasher::new();
    id.hash(&mut h);
    format!("row-{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::{LIST_HEIGHT_BUDGET, visible_rows};

    #[test]
    fn themed_lists_keep_the_original_height_budget() {
        assert_eq!(visible_rows(52.0), 8);
        assert_eq!(visible_rows(44.0), 8);
        for height in [52.0, 64.0, 72.0, 92.0, 100.0] {
            assert!(visible_rows(height) as f32 * height <= LIST_HEIGHT_BUDGET);
        }
    }
}
