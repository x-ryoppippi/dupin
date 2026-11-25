use std::hash::{DefaultHasher, Hash, Hasher};

use dupin_core::{Accessory, Item, View};
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

/// The height of a section's heading row.
pub(super) const SECTION_H: f32 = 30.0;

/// The height of a long list whose selected row shows its detail, as tall as a list of cards.
const CARDS_MAX_H: f32 = 440.0;

/// How the result list lays out its rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    /// One row per result, only those in view laid out.
    Windowed,
    /// One row per result, every one laid out: headings break the uniform row height that
    /// windowing needs, and sectioned lists are short (a provider's own results).
    Sectioned,
    /// Cards and rows, every one laid out.
    Cards,
    /// A long list of rows, only those in view laid out, whose selected row shows its detail.
    WindowedCard,
}

fn layout(s: &Ui, items: &[Item]) -> Layout {
    let text_only = items.iter().all(|item| matches!(item.view, View::Text { .. }));
    let cards = !text_only || (0..items.len()).any(|index| s.session.detail(index).is_some());
    let headings = (0..items.len()).any(|index| s.session.section_start(index).is_some());
    let long = items.len() > 2 * visible_rows(s.theme.row_height as f32);
    match (cards, headings) {
        (false, false) => Layout::Windowed,
        (false, true) => Layout::Sectioned,
        // Only the selected row of a long list of text rows shows a card: the rest stay rows,
        // so the list can still lay out only what is in view, however many rows it holds.
        (true, false) if text_only && long && only_selected_shows_detail(s, items) => Layout::WindowedCard,
        (true, _) => Layout::Cards,
    }
}

fn only_selected_shows_detail(s: &Ui, items: &[Item]) -> bool {
    let selected = s.session.selected();
    (0..items.len()).all(|index| index == selected || s.session.detail(index).is_none())
}

/// The rows to load icons for: those in view and a screenful either side, and the selected
/// row's, while the list lays out only what is in view; otherwise, every row. `in_view` is the
/// range the windowed list last reported.
pub fn wanted_rows(s: &Ui, in_view: std::ops::Range<usize>) -> Vec<usize> {
    let items = s.session.items();
    if !matches!(layout(s, items), Layout::Windowed | Layout::WindowedCard) {
        return (0..items.len()).collect();
    }
    let screen = visible_rows(s.theme.row_height as f32);
    let around = |range: std::ops::Range<usize>| {
        range.start.saturating_sub(screen)..range.end.saturating_add(screen).min(items.len())
    };
    let selected = s.session.selected();
    let mut rows: Vec<usize> = around(in_view).chain(around(selected..selected + 1)).collect();
    rows.sort_unstable();
    rows.dedup();
    rows
}

pub(super) fn results(s: &Ui, items: &[Item], t: &ResolvedTheme, accent: Color) -> El {
    // Rows without icons only keep the icon column when another row in the list has one, so
    // titles still line up.
    let gutter = items.iter().any(|item| item.icon.is_some());
    let layout = layout(s, items);
    if layout == Layout::WindowedCard {
        let selected = s.session.selected();
        let rows: Vec<El> = items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let on = index == selected;
                let row = match s.session.detail(index) {
                    Some(detail) => super::detail::detail_card(s, item, detail, index, on, t, accent),
                    None => result_row(s, item, index, on, gutter, t, accent),
                };
                row.key(row_key(&s.session.layout_key(index)))
            })
            .collect();
        let list = col()
            .key("rows")
            .w_full()
            .growth(GrowthSpec::default())
            .virtualize(s.theme.row_height as f32)
            .on_visible_rows(Ui::rows_in_view)
            .children(rows);
        return col().key("results").w_full().py(LIST_PAD).child(
            col()
                .key("scroll")
                .w_full()
                .h(CARDS_MAX_H)
                .scroll(ScrollSpec { stick_bottom: false })
                .instant_changes()
                .child(list),
        );
    }
    if layout == Layout::Cards {
        let rows = items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let on = index == s.session.selected();
                let row = if let Some(detail) = s.session.detail(index) {
                    super::detail::detail_card(s, item, detail, index, on, t, accent)
                } else {
                    match &item.view {
                        View::Text { .. } => result_row(s, item, index, on, gutter, t, accent),
                        View::Detail { .. } | View::Pair { .. } => {
                            content_card(s, item, index, on, t, accent)
                        }
                    }
                };
                sectioned(s, index, row, t)
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
    let row_height = s.theme.row_height as f32;
    let selected = s.session.selected();
    let rows: Vec<El> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let row = result_row(s, item, i, i == selected, gutter, t, accent);
            sectioned(s, i, row, t)
        })
        .collect();
    let headings = (0..items.len())
        .filter(|&index| s.session.section_start(index).is_some())
        .count();
    let list = col().key("rows").w_full().growth(GrowthSpec::default());
    let (height, list) = if layout == Layout::Windowed {
        let shown = items.len().min(visible_rows(row_height)) as f32;
        (
            shown * row_height,
            list.virtualize(row_height).on_visible_rows(Ui::rows_in_view),
        )
    } else {
        let full = items.len() as f32 * row_height + headings as f32 * SECTION_H;
        (full.min(visible_rows(row_height) as f32 * row_height), list)
    };
    col().key("results").w_full().py(LIST_PAD).child(
        col()
            .key("scroll")
            .w_full()
            .h(height)
            .scroll(ScrollSpec { stick_bottom: false })
            .instant_changes()
            .child(list.children(rows)),
    )
}

/// `result` under its section's heading when it starts one. The heading shares the row's key,
/// so revealing the row reveals its heading too.
fn sectioned(s: &Ui, index: usize, result: El, t: &ResolvedTheme) -> El {
    let Some(section) = s.session.section_start(index) else {
        return result.key(row_key(&s.session.layout_key(index)));
    };
    let key = row_key(&s.session.layout_key(index));
    col()
        .key(key.clone())
        .w_full()
        .child(
            row()
                .key(format!("{key}-section"))
                .w_full()
                .h(SECTION_H)
                .px(LIST_PAD + 10.0)
                .pt(8.0)
                .center_y()
                .role("heading")
                .child(
                    text(section)
                        .font(t.font(type_::SECTION))
                        .color(t.text_secondary)
                        .truncate(),
                ),
        )
        .child(result.key(format!("{key}-row")))
}

fn content_card(s: &Ui, item: &Item, index: usize, on: bool, t: &ResolvedTheme, accent: Color) -> El {
    let title = match &item.view {
        View::Detail { title, .. } => Some(title.as_str()),
        _ => None,
    };
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
    card_slot(card(s, item, index, on, title, t, accent).child(body))
}

pub(super) fn card_slot(card: El) -> El {
    col().w_full().px(LIST_PAD).py(LIST_PAD).child(card)
}

/// A selectable card for a result that shows more than one line: its title, if any, beside
/// the Actions link, above whatever the caller adds.
pub(super) fn card(
    s: &Ui,
    item: &Item,
    index: usize,
    on: bool,
    title: Option<&str>,
    t: &ResolvedTheme,
    accent: Color,
) -> El {
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
    header = match title {
        Some(title) => header.child(
            text(title)
                .font(t.font(TextStyle::new(24.0, 600.0)))
                .color(t.text)
                .grow(1.0)
                .min_w(0.0)
                .wrap(),
        ),
        None => header.child(el().grow(1.0)),
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
    if title.is_some() || item.has_actions() {
        card = card.child(header);
    }
    card
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

fn result_row(
    s: &Ui,
    item: &Item,
    index: usize,
    on: bool,
    gutter: bool,
    t: &ResolvedTheme,
    accent: Color,
) -> El {
    let View::Text { subtitle, .. } = &item.view else {
        unreachable!("text row")
    };
    let icon = item_icon(
        s,
        item.icon.as_ref(),
        gutter,
        if on { t.selected_text } else { t.text },
    );
    let id = s.session.result_key(index);
    text_row(
        t,
        accent,
        s.theme.row_height as f32,
        on,
        icon,
        RowLabels {
            title: title(
                item,
                if on { t.selected_text } else { t.text },
                if on { t.selected_text } else { accent },
                t,
            ),
            subtitle,
            accessories: s.session.accessories(index),
        },
    )
    .on_press_tx(move |s: &mut Ui, tx| s.open(&id, tx))
}

/// The icon, an empty slot while it loads, or nothing when there is no icon and `gutter` is
/// false. A symbol is drawn in `ink`, the colour of the row's title.
pub(super) fn item_icon(s: &Ui, icon: Option<&dupin_core::Icon>, gutter: bool, ink: Color) -> Option<El> {
    if icon.is_none() && !gutter {
        return None;
    }
    let slot = el().size(ICON_SIZE, ICON_SIZE);
    if let Some(dupin_core::Icon::Symbol(name)) = icon {
        return Some(slot.pixels(&crate::symbols::symbol(name, crate::symbols::ROW, ink)));
    }
    Some(match (icon, icon.and_then(|icon| s.icons.get(icon))) {
        // A picture keeps its own shape inside the icon's square, as Finder's thumbnails do.
        (Some(dupin_core::Icon::Host(_)), Some(image)) => {
            slot.radius(4.0).pixels_fit(image.pixels(), rune::SurfaceFit::Fit)
        }
        (_, Some(image)) => slot.pixels(image.pixels()),
        (_, None) => slot,
    })
}

/// What a text row says: its title, its subtitle beneath and its accessories at its end.
pub(super) struct RowLabels<'a> {
    pub title: El,
    pub subtitle: &'a str,
    pub accessories: &'a [Accessory],
}

pub(super) fn text_row(
    t: &ResolvedTheme,
    accent: Color,
    height: f32,
    on: bool,
    icon: Option<El>,
    labels: RowLabels<'_>,
) -> El {
    let RowLabels {
        title,
        subtitle,
        accessories,
    } = labels;
    let ink = if on { t.selected_text } else { t.text_secondary };
    let accessories = accessories.iter().map(|accessory| match accessory {
        Accessory::Text(text) => rune::text(text).font(t.font(type_::LABEL)).color(ink).truncate(),
        Accessory::Tag(tag) => row()
            .h(22.0)
            .px(8.0)
            .radius(6.0)
            .center_y()
            .fill(if on {
                t.selected_text.alpha(0.2)
            } else {
                accent.alpha(0.16)
            })
            .child(
                rune::text(tag)
                    .font(t.font(type_::LABEL))
                    .color(if on { t.selected_text } else { accent })
                    .truncate(),
            ),
    });
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
            .children(icon)
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
            )
            .children(accessories),
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
