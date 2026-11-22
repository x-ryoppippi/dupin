//! The panel: a query field over a virtualised result list.

use std::hash::{DefaultHasher, Hash, Hasher};

use dupin_core::session::Results;
use dupin_core::{Item, View};
use rune::tok::{self, type_};
use rune::{
    Align, Color, Cx, El, GrowthSpec, ScrollSpec, Span, SystemMaterial, TextStyle, col, effect, el, rich,
    row, text, text_input,
};

use crate::ui::{ICON_SIZE, Ui};

pub const RADIUS: f32 = 14.0;
pub const HEADER_H: f32 = 64.0;
const ROW_H: f32 = 52.0;
/// Rows shown before the list scrolls.
const VISIBLE_ROWS: usize = 8;
const LIST_PAD: f32 = 6.0;
/// How much of the theme's card color covers the glass.
const FROST: f32 = 0.45;
const WARNING: Color = Color {
    r: 0.95,
    g: 0.55,
    b: 0.15,
    a: 0.14,
};

pub fn view(s: &Ui, cx: &mut Cx<Ui>) -> El {
    let dark = cx.system_dark();
    let t = tok::theme(dark);
    let accent = tok::accent(0, dark);

    // A light frost over the glass keeps text readable over whatever is behind the panel.
    let ink = if dark { Color::WHITE } else { Color::BLACK };
    let mut panel = col()
        .key("frost")
        .w_full()
        .radius(RADIUS)
        .continuous_corners()
        .clip_rounded(RADIUS)
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
    panel = panel.child(header(s, &t, accent));
    let notices = [
        ("config-problem", s.config_problem.as_deref()),
        ("notice", s.notice.as_deref()),
        ("registration-notice", s.registration_notice.as_deref()),
        ("backdrop-notice", s.backdrop.problem()),
    ];
    for (key, notice) in notices.into_iter().filter_map(|(k, n)| Some((k, n?))) {
        panel = panel.child(divider(&t)).child(
            row().key(key).w_full().px(18.0).py(12.0).fill(WARNING).child(
                text(notice)
                    .font(type_::BODY.lh(18.0))
                    .color(t.text)
                    .wrap()
                    .w_full(),
            ),
        );
    }
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
                    .font(type_::BODY)
                    .color(t.text_secondary)
                    .wrap()
                    .w_full(),
                ),
        );
    }
    let items = s.session.items();
    if !items.is_empty()
        && crate::commands::query_problem(s.session.command_id(), s.session.query()).is_none()
    {
        panel = panel.child(divider(&t)).child(results(s, items, &t, accent));
    } else if let Some(hint) = hint(s) {
        panel = panel.child(divider(&t)).child(
            row()
                .key("hint")
                .w_full()
                .h(44.0)
                .px(20.0)
                .center_y()
                .child(text(hint).font(type_::BODY).color(t.text_secondary)),
        );
    }
    // Liquid Glass (NSGlassEffectView) under the panel on macOS 26 and later. Headless renders
    // and opaque windows draw the fill instead, so screenshots show the plain card.
    col().w_full().child(
        col()
            .key("panel")
            .w_full()
            .radius(RADIUS)
            .continuous_corners()
            .fill(t.card)
            .system_material(SystemMaterial::Glass)
            .child(panel),
    )
}

fn header(s: &Ui, t: &tok::Theme, accent: Color) -> El {
    let presentation = &s.session.command().presentation;
    let placeholder = presentation.placeholder.as_str();
    let field = text_input(&s.query, TextStyle::new(22.0, 400.0).lh(30.0))
        .key("query")
        .label(placeholder)
        .single_line()
        .grow(1.0)
        .color(t.text)
        .pill_colors(accent.alpha(0.16), Some(accent))
        .placeholder(placeholder, t.text_secondary)
        .on_edit(|s: &mut Ui| &mut s.query)
        .on_input_key(|s: &mut Ui, ev, tx| s.key(ev, tx))
        .on_input_change(|s: &mut Ui, tx| s.edited(tx));
    let mut bar = row()
        .key("header")
        .w_full()
        .h(HEADER_H)
        .px(18.0)
        .gap(12.0)
        .center_y();
    bar = if let Some(chip) = presentation.chip.as_deref() {
        bar.child(
            row()
                .key("mode")
                .h(28.0)
                .px(10.0)
                .radius(7.0)
                .center_y()
                .fill(accent.alpha(0.16))
                .child(text(chip).font(type_::TITLE).color(accent)),
        )
    } else {
        bar.child(
            text("⌕")
                .font(TextStyle::new(24.0, 400.0).lh(30.0))
                .color(t.text_secondary),
        )
    };
    bar = bar.child(field);
    if presentation.show_searching && matches!(s.session.results(), Results::Searching { .. }) {
        bar = bar.child(text("Searching…").font(type_::LABEL).color(t.text_secondary));
    }
    bar
}

fn hint(s: &Ui) -> Option<&str> {
    if let Some(problem) = crate::commands::query_problem(s.session.command_id(), s.session.query()) {
        return Some(problem);
    }
    let presentation = &s.session.command().presentation;
    if s.session.query().trim().is_empty() {
        presentation.empty_hint.as_deref()
    } else if s.session.results() == Results::Done && s.session.failures().next().is_none() {
        presentation.no_results_hint.as_deref()
    } else {
        None
    }
}

fn results(s: &Ui, items: &[Item], t: &tok::Theme, accent: Color) -> El {
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
    let shown = items.len().min(VISIBLE_ROWS) as f32;
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
            .h(shown * ROW_H)
            .scroll(ScrollSpec { stick_bottom: false })
            .instant_changes()
            .child(
                col()
                    .key("rows")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtualize(ROW_H)
                    .children(rows),
            ),
    )
}

fn content_card(s: &Ui, item: &Item, index: usize, on: bool, t: &tok::Theme, accent: Color) -> El {
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
                .font(TextStyle::new(24.0, 600.0))
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
    if !item.actions.is_empty() {
        header = header.child(
            text("→ Actions")
                .font(type_::BODY)
                .color(accent)
                .cursor(rune::Cursor::Pointer)
                .hover(t.hover)
                .on_press_tx(move |s: &mut Ui, tx| s.actions(&id, tx)),
        );
    }
    if has_title || !item.actions.is_empty() {
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
                    .font(TextStyle::new(28.0, 400.0))
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

fn pair_value(id: &str, part: &str, value: &str, label: &str, t: &tok::Theme) -> El {
    col()
        .grow(1.0)
        .min_w(0.0)
        .gap(6.0)
        .child(
            text(value)
                .key(format!("{}-{part}", row_key(id)))
                .w_full()
                .font(TextStyle::new(28.0, 600.0).lh(36.0))
                .color(t.text)
                .align(Align::Center)
                .wrap()
                .selectable(true),
        )
        .child(
            text(label)
                .w_full()
                .font(type_::LABEL)
                .color(t.text_secondary)
                .align(Align::Center),
        )
}

fn detail_body(id: &str, body: &str, t: &tok::Theme) -> El {
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
                .font(TextStyle::new(16.0, 400.0).lh(25.0))
                .color(t.text)
                .wrap()
                .selectable(true)
        }))
}

fn result_row(s: &Ui, item: &Item, index: usize, on: bool, t: &tok::Theme, accent: Color) -> El {
    let View::Text { subtitle, .. } = &item.view else {
        unreachable!("text row")
    };
    let (ink, sub) = if on {
        (Color::WHITE, Color::WHITE.alpha(0.78))
    } else {
        (t.text, t.text_secondary)
    };
    let icon = match s.icons.get(&s.session.layout_key(index)) {
        Some(image) => el().size(ICON_SIZE, ICON_SIZE).pixels(image.pixels()),
        None => el().size(ICON_SIZE, ICON_SIZE),
    };
    let id = s.session.result_key(index);
    // Rows of a virtual list keep their padding inside, not in gaps between them.
    row()
        .key(row_key(&s.session.layout_key(index)))
        .w_full()
        .h(ROW_H)
        .px(LIST_PAD)
        .child(
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
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(title(item, ink, if on { Color::WHITE } else { accent }))
                        .child(text(subtitle.as_str()).font(type_::LABEL).color(sub).truncate()),
                )
                .on_press_tx(move |s: &mut Ui, tx| s.open(&id, tx)),
        )
}

/// The title with the characters the query matched in bold.
fn title(item: &Item, ink: Color, highlight: Color) -> El {
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
        .font(TextStyle::new(15.0, 500.0).lh(20.0))
        .color(ink)
        .truncate()
}

fn divider(t: &tok::Theme) -> El {
    el().h(1.0).w_full().fill(t.separator)
}

/// Element keys for result rows. Ids are paths, whose slashes would read as key paths in
/// scripts and `scroll_into_view`, so rows are keyed by a hash of the id instead.
pub fn row_key(id: &str) -> String {
    let mut h = DefaultHasher::new();
    id.hash(&mut h);
    format!("row-{:016x}", h.finish())
}
