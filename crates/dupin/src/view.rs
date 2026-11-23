//! The panel: a query field over a virtualised result list.

use std::hash::{DefaultHasher, Hash, Hasher};

use dupin_core::session::Results;
use dupin_core::{Item, View};
use rune::tok::{self, type_};
use rune::{
    Align, Color, Cx, El, GrowthSpec, ScrollSpec, Span, SystemMaterial, TextStyle, col, effect, el, rich,
    row, text, text_input,
};

use crate::ui::{ActionStage, ApplicationState, ICON_SIZE, Ui};

pub const HEADER_H: f32 = 64.0;
/// Rows shown before the list scrolls.
const VISIBLE_ROWS: usize = 8;
const LIST_HEIGHT_BUDGET: f32 = 416.0;

fn visible_rows(row_height: f32) -> usize {
    VISIBLE_ROWS
        .min((LIST_HEIGHT_BUDGET / row_height).floor() as usize)
        .max(1)
}
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
    if let Some(picker) = &s.picker {
        let field = text_input(&s.query, TextStyle::new(22.0, 400.0))
            .key("pick-query")
            .single_line()
            .color(Color::WHITE)
            .on_edit(|s: &mut Ui| &mut s.query)
            .on_input_key(Ui::key)
            .on_input_change(Ui::edited);
        return picker_view(
            field,
            &picker.values,
            &s.query.text,
            picker.selected,
            s.theme.row_height as f32,
        )
        .on_key(Ui::key);
    }
    let dark = cx.system_dark();
    let t = ResolvedTheme::new(&s.theme, dark);
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
    panel = panel.child(if s.action_screen.is_some() {
        action_header(s, &t)
    } else {
        header(s, &t, accent)
    });
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
                    .font(t.font(type_::BODY.lh(18.0)))
                    .color(t.text)
                    .wrap()
                    .w_full(),
            ),
        );
    }
    use dupin_mac::window::DisplayNotice;
    if let Some(notice) = s.display_notice {
        let message = match notice {
            DisplayNotice::AccessibilityRequired => {
                "Dupin needs Accessibility access to find the focused window. Using the main display."
            }
            DisplayNotice::FocusedWindowUnavailable => {
                "Couldn't find the focused window's display. Using the main display."
            }
        };
        panel = panel.child(divider(&t)).child(
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
        panel = panel.child(divider(&t)).child(
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
    let items = s.session.items();
    if let Some(renaming) = s.session.renaming() {
        panel = panel.child(divider(&t)).child(rename_status(s, renaming, &t));
    } else if s.action_screen.is_some() {
        panel = panel.child(divider(&t)).child(action_list(s, &t, accent));
    } else if !items.is_empty() && query_problem(s).is_none() {
        panel = panel.child(divider(&t)).child(results(s, items, &t, accent));
    } else if let Some(hint) = hint(s) {
        panel = panel.child(divider(&t)).child(
            row()
                .key("hint")
                .w_full()
                .h(44.0)
                .px(20.0)
                .center_y()
                .child(text(hint).font(t.font(type_::BODY)).color(t.text_secondary)),
        );
    }
    if s.session.destination().is_some() {
        panel = panel.child(divider(&t)).child(
            row().w_full().px(20.0).py(8.0).child(
                text("↩ Choose folder · esc Cancel")
                    .font(t.font(type_::LABEL))
                    .color(t.text_secondary),
            ),
        );
    }
    // Liquid Glass (NSGlassEffectView) under the panel on macOS 26 and later. Headless renders
    // and opaque windows draw the fill instead, so screenshots show the plain card.
    col().w_full().child(
        col()
            .key("panel")
            .w_full()
            .radius(s.active_radius)
            .continuous_corners()
            .fill(t.card)
            .system_material(SystemMaterial::Glass)
            .child(panel),
    )
}

fn picker_view(field: El, values: &[serde_json::Value], query: &str, selected: usize, row_height: f32) -> El {
    let shown = visible_rows(row_height);
    let mut list = col().key("picker").w_full().fill(Color::BLACK).child(
        row()
            .w_full()
            .h(HEADER_H)
            .px(16.0)
            .center_y()
            .child(field.grow(1.0)),
    );
    for (index, original) in crate::ui::pick_indices(values, query)
        .into_iter()
        .enumerate()
        .skip(selected.saturating_sub(shown - 1))
        .take(shown)
    {
        let value = &values[original];
        let label = value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| value.to_string());
        list = list.child(
            row()
                .key(format!("pick-item-{original}"))
                .w_full()
                .h(row_height)
                .px(16.0)
                .center_y()
                .fill(if index == selected {
                    Color {
                        r: 0.2,
                        g: 0.27,
                        b: 0.4,
                        a: 1.0,
                    }
                } else {
                    Color::BLACK
                })
                .child(text(label).color(Color::WHITE)),
        );
    }
    list
}

fn action_header(s: &Ui, t: &ResolvedTheme) -> El {
    let screen = s.action_screen.as_ref().unwrap();
    let icon = match s.icons.get(&screen.layout_key) {
        Some(image) => el().size(ICON_SIZE, ICON_SIZE).pixels(image.pixels()),
        None => el().size(ICON_SIZE, ICON_SIZE),
    };
    let subtitle = match &screen.item.view {
        View::Text { subtitle, .. } => subtitle.as_str(),
        _ => "Actions",
    };
    row()
        .key("action-context")
        .w_full()
        .h(HEADER_H)
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
                    text(screen.item.display_title())
                        .font(t.font(type_::TITLE))
                        .color(t.text)
                        .truncate(),
                )
                .child(
                    text(subtitle)
                        .font(t.font(type_::LABEL))
                        .color(t.text_secondary)
                        .truncate(),
                ),
        )
        .child(
            text(if matches!(screen.stage, ActionStage::List) {
                "← Search"
            } else {
                "← Actions"
            })
            .font(t.font(type_::LABEL))
            .color(t.text_secondary)
            .on_press_tx(|s: &mut Ui, tx| {
                s.action_back(tx);
            }),
        )
}

fn application_locations(apps: &[dupin_mac::workspace::Application]) -> Vec<Option<String>> {
    let names: Vec<_> = apps.iter().map(|app| app.name.to_lowercase()).collect();
    let mut counts = std::collections::HashMap::new();
    for name in &names {
        *counts.entry(name).or_insert(0) += 1;
    }
    apps.iter()
        .zip(&names)
        .map(|(app, name)| {
            if counts[name] > 1 {
                app.path
                    .parent()
                    .map(|parent| dupin_core::item::tilde(&parent.to_string_lossy()))
            } else {
                None
            }
        })
        .collect()
}

fn action_list(s: &Ui, t: &ResolvedTheme, accent: Color) -> El {
    let screen = s.action_screen.as_ref().unwrap();
    match &screen.stage {
        ActionStage::Applications { state, selected, .. } => {
            let mut list = col().key("applications").w_full().py(LIST_PAD);
            match state {
                ApplicationState::Loading => {
                    list = list.child(text("Finding apps…").color(t.text_secondary).padding(16.0))
                }
                ApplicationState::Failed(error) => {
                    list = list.child(text(error).color(t.text_secondary).padding(16.0).wrap())
                }
                ApplicationState::Ready(apps) if apps.is_empty() => {
                    list = list.child(
                        text("No apps can open this item")
                            .color(t.text_secondary)
                            .padding(16.0),
                    )
                }
                ApplicationState::Ready(apps) => {
                    let locations = application_locations(apps);
                    list = list.child(
                        col()
                            .w_full()
                            .max_h(LIST_HEIGHT_BUDGET)
                            .scroll(ScrollSpec { stick_bottom: false })
                            .children(apps.iter().enumerate().map(|(index, app)| {
                                let on = index == *selected;
                                let icon = s
                                    .icons
                                    .get(&format!("choice-{}", app.path.display()))
                                    .map(|image| el().size(ICON_SIZE, ICON_SIZE).pixels(image.pixels()))
                                    .unwrap_or_else(|| el().size(ICON_SIZE, ICON_SIZE));
                                row()
                                    .key(format!("action-{index}"))
                                    .w_full()
                                    .h(44.0)
                                    .px(12.0)
                                    .gap(12.0)
                                    .center_y()
                                    .radius(8.0)
                                    .fill(if on { accent } else { Color::TRANSPARENT })
                                    .hover(if on { accent } else { t.hover })
                                    .child(icon)
                                    .child(
                                        col()
                                            .grow(1.0)
                                            .min_w(0.0)
                                            .child(
                                                text(&app.name)
                                                    .font(t.font(type_::BODY))
                                                    .color(if on { t.selected_text } else { t.text })
                                                    .truncate(),
                                            )
                                            .children(locations[index].as_deref().map(|location| {
                                                text(location)
                                                    .font(t.font(type_::LABEL))
                                                    .color(if on {
                                                        t.selected_text
                                                    } else {
                                                        t.text_secondary
                                                    })
                                                    .truncate()
                                            })),
                                    )
                                    .child(
                                        text(if app.default { "Default" } else { "" })
                                            .font(t.font(type_::LABEL))
                                            .color(t.text_secondary),
                                    )
                                    .on_press_tx(move |s: &mut Ui, tx| s.pick_application(index, tx))
                            })),
                    );
                }
            }
            return list;
        }
        ActionStage::Confirm { action_id, selected } => {
            let action = screen.item.item_action(action_id).unwrap();
            let confirmation = action.confirmation().unwrap();
            let path = match screen.item.action(action_id) {
                Some(dupin_core::Action::Trash(path)) => path.to_string_lossy(),
                _ => "".into(),
            };
            return col()
                .key("confirmation")
                .w_full()
                .py(LIST_PAD)
                .child(
                    col()
                        .padding(16.0)
                        .gap(6.0)
                        .child(
                            text(&confirmation.prompt)
                                .font(t.font(type_::BODY))
                                .color(t.text)
                                .wrap(),
                        )
                        .children((!path.is_empty()).then(|| {
                            text(path.as_ref())
                                .font(t.font(type_::LABEL))
                                .color(t.text_secondary)
                                .wrap()
                        })),
                )
                .children(
                    ["Cancel", confirmation.button.as_str()]
                        .into_iter()
                        .enumerate()
                        .map(|(index, title)| {
                            let on = index == *selected;
                            let red = t.destructive;
                            row()
                                .key(format!("action-{index}"))
                                .w_full()
                                .h(44.0)
                                .px(12.0)
                                .center_y()
                                .radius(8.0)
                                .fill(if on {
                                    if index == 1 { red } else { accent }
                                } else {
                                    Color::TRANSPARENT
                                })
                                .child(text(title).font(t.font(type_::BODY)).color(if on {
                                    t.selected_text
                                } else if index == 1 {
                                    red
                                } else {
                                    t.text
                                }))
                                .on_press_tx(move |s: &mut Ui, tx| s.select_confirmation(index, tx))
                        }),
                );
        }
        ActionStage::List => {}
    }
    col().key("actions").w_full().py(LIST_PAD).child(
        col()
            .w_full()
            .max_h((s.theme.row_height as f32 - 8.0) * visible_rows(s.theme.row_height as f32 - 8.0) as f32)
            .scroll(ScrollSpec { stick_bottom: false })
            .children(screen.item.actions.iter().enumerate().map(|(index, action)| {
                let on = index == screen.selected;
                let key = screen.key.clone();
                let id = action.id.clone();
                row()
                    .key(format!("action-{index}"))
                    .w_full()
                    .h(s.theme.row_height as f32 - 8.0)
                    .px(LIST_PAD)
                    .child(
                        row()
                            .w_full()
                            .h_full()
                            .px(12.0)
                            .gap(12.0)
                            .center_y()
                            .radius(8.0)
                            .fill(if on { accent } else { Color::TRANSPARENT })
                            .hover(if on { accent } else { t.hover })
                            .child(
                                text(&action.title)
                                    .font(t.font(type_::BODY))
                                    .color(if on {
                                        t.selected_text
                                    } else if action.confirmation().is_some() {
                                        t.destructive
                                    } else {
                                        t.text
                                    })
                                    .grow(1.0),
                            )
                            .child(
                                text(if on { "↩" } else { "" })
                                    .font(t.font(type_::LABEL))
                                    .color(t.selected_text.alpha(0.78)),
                            )
                            .on_press_tx(move |s: &mut Ui, tx| s.pick_action(&key, &id, tx)),
                    )
            })),
    )
}

fn header(s: &Ui, t: &ResolvedTheme, accent: Color) -> El {
    let presentation = &s.session.command().presentation;
    let renaming = s.session.renaming().is_some();
    let destination = s.session.destination();
    let placeholder = if renaming {
        "Name"
    } else if destination.is_some() {
        "Search folders"
    } else {
        presentation.placeholder.as_str()
    };
    let field = text_input(&s.query_display(), t.font(TextStyle::new(22.0, 400.0).lh(30.0)))
        .key("query")
        .label(placeholder)
        .single_line()
        .grow(1.0)
        .min_w(0.0)
        .color(t.text)
        .pill_colors(accent.alpha(0.16), Some(accent))
        .placeholder(placeholder, t.text_secondary)
        .on_edit(Ui::query_edit)
        .on_input_key(|s: &mut Ui, ev, tx| s.key(ev, tx))
        .on_input_change(|s: &mut Ui, tx| s.edited(tx));
    let mut bar = row()
        .key("header")
        .w_full()
        .h(HEADER_H)
        .px(18.0)
        .gap(12.0)
        .center_y();
    let destination_chip = destination.map(|picker| {
        let operation = match picker.transfer() {
            dupin_core::item::Transfer::Move => "Move to",
            dupin_core::item::Transfer::Copy => "Copy to",
        };
        let name = picker.path().file_name().unwrap_or(picker.path().as_os_str());
        format!("{operation} · {}", name.to_string_lossy())
    });
    let chip = if let Some(chip) = destination_chip.as_deref() {
        Some(chip)
    } else if renaming {
        Some("Rename")
    } else {
        presentation.chip.as_deref()
    };
    bar = if let Some(chip) = chip {
        bar.child(
            row()
                .key("mode")
                .max_w(280.0)
                .h(28.0)
                .px(10.0)
                .radius(7.0)
                .center_y()
                .fill(accent.alpha(0.16))
                .child(text(chip).font(t.font(type_::TITLE)).color(accent).truncate()),
        )
    } else {
        bar.child(
            text("⌕")
                .font(t.font(TextStyle::new(24.0, 400.0).lh(30.0)))
                .color(t.text_secondary),
        )
    };
    bar = bar.child(field);
    if !renaming && presentation.show_searching && matches!(s.session.results(), Results::Searching { .. }) {
        bar = bar.child(
            text("Searching…")
                .font(t.font(type_::LABEL))
                .color(t.text_secondary),
        );
    }
    bar
}

/// What is being renamed, and what Enter will do with the name typed so far.
fn rename_status(s: &Ui, renaming: &dupin_core::session::Renaming, t: &ResolvedTheme) -> El {
    let icon = match s.icons.get(renaming.layout_key()) {
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

fn query_problem(s: &Ui) -> Option<&'static str> {
    let query = s.session.query();
    if s.session.destination().is_some()
        && dupin_core::file_query::FileQuery::parse(query).is_ok_and(|query| {
            query
                .kind
                .is_some_and(|kind| kind != dupin_core::file_query::FileKind::Folder)
        })
    {
        return Some("Choose a folder. Use kind:folder or remove the kind filter.");
    }
    crate::commands::query_problem(s.session.command_id(), query)
}

fn hint(s: &Ui) -> Option<&str> {
    if let Some(problem) = query_problem(s) {
        return Some(problem);
    }
    if s.session.destination().is_some() {
        return (s.session.results() == Results::Done && s.session.failures().next().is_none())
            .then_some("No matching folders");
    }
    let presentation = &s.session.command().presentation;
    let empty = s.session.query().trim().is_empty();
    match s.session.results() {
        Results::Idle if empty => presentation.empty_hint.as_deref(),
        Results::Done if s.session.failures().next().is_none() => {
            if empty {
                presentation.empty_hint.as_deref()
            } else {
                presentation.no_results_hint.as_deref()
            }
        }
        _ => None,
    }
}

fn results(s: &Ui, items: &[Item], t: &ResolvedTheme, accent: Color) -> El {
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
    if !item.actions.is_empty() {
        header = header.child(
            text("→ Actions")
                .font(t.font(type_::BODY))
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
    let icon = match s.icons.get(&s.session.layout_key(index)) {
        Some(image) => el().size(ICON_SIZE, ICON_SIZE).pixels(image.pixels()),
        None => el().size(ICON_SIZE, ICON_SIZE),
    };
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

fn text_row(
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

fn divider(t: &ResolvedTheme) -> El {
    el().h(1.0).w_full().fill(t.separator)
}

/// Element keys for result rows. Ids are paths, whose slashes would read as key paths in
/// scripts and `scroll_into_view`, so rows are keyed by a hash of the id instead.
pub fn row_key(id: &str) -> String {
    let mut h = DefaultHasher::new();
    id.hash(&mut h);
    format!("row-{:016x}", h.finish())
}

pub struct ResolvedTheme {
    palette: tok::Theme,
    pub accent: Color,
    pub selected_text: Color,
    pub destructive: Color,
    scale: f32,
    family: tok::Family,
}
impl std::ops::Deref for ResolvedTheme {
    type Target = tok::Theme;
    fn deref(&self) -> &Self::Target {
        &self.palette
    }
}
impl ResolvedTheme {
    pub fn new(theme: &dupin_core::config::Theme, dark: bool) -> Self {
        let c = if dark { &theme.dark } else { &theme.light };
        let mut palette = tok::theme(dark);
        fn color(value: &Option<String>, fallback: Color) -> Color {
            value
                .as_ref()
                .and_then(|v| rune::color::hex(v))
                .unwrap_or(fallback)
        }
        palette.card = color(&c.background, palette.card);
        palette.text = color(&c.text, palette.text);
        palette.text_secondary = color(&c.secondary, palette.text_secondary);
        palette.hover = color(&c.hover, palette.hover);
        palette.separator = color(&c.separator, palette.separator);
        Self {
            palette,
            accent: color(&c.accent, tok::accent(0, dark)),
            selected_text: color(&c.selected_text, Color::WHITE),
            destructive: rune::color::hex(if dark { "#FF453A" } else { "#FF3B30" }).unwrap(),
            scale: theme.font_size as f32 / 15.0,
            family: match theme.font_family {
                dupin_core::config::FontFamily::System => tok::Family::Sans,
                dupin_core::config::FontFamily::Monospace => tok::Family::SystemMono,
            },
        }
    }
    pub fn font(&self, mut style: TextStyle) -> TextStyle {
        style.size *= self.scale;
        style.line_height *= self.scale;
        style.family = self.family;
        style
    }
}

fn preview_geometry(theme: &dupin_core::config::Theme, avail_w: f32) -> (f32, f32, f32, usize) {
    let n = if theme.row_height <= 72 { 3 } else { 2 };
    let h = HEADER_H + 1.0 + 2.0 * LIST_PAD + n as f32 * theme.row_height as f32;
    let w = theme.panel_width as f32;
    // Reserve room for the view toggle and exact-value caption in the well.
    let scale = (avail_w / w).min(104.0 / h).min(0.5);
    (w, h, scale, n)
}

pub fn theme_preview(theme: &dupin_core::config::Theme, dark: bool, avail_w: f32) -> El {
    let t = ResolvedTheme::new(theme, dark);
    let (w, h, scale, n) = preview_geometry(theme, avail_w);
    let radius = theme.corner_radius as f32;
    let panel = col()
        .w(w)
        .h(h)
        .radius(radius)
        .continuous_corners()
        .clip_rounded(radius)
        .fill(t.card)
        .border(1.0, t.text.alpha(0.08))
        .child(
            row()
                .w_full()
                .h(HEADER_H)
                .px(18.0)
                .gap(12.0)
                .center_y()
                .child(
                    text("⌕")
                        .font(t.font(TextStyle::new(24.0, 400.0).lh(30.0)))
                        .color(t.text_secondary),
                )
                .child(
                    text("gh dupin")
                        .font(t.font(TextStyle::new(22.0, 400.0).lh(30.0)))
                        .color(t.text),
                )
                .child(el().size(2.0, 26.0).fill(t.accent)),
        )
        .child(divider(&t))
        .child(col().w_full().py(LIST_PAD).children((0..n).map(|i| {
            let on = i == 0;
            text_row(
                &t,
                t.accent,
                theme.row_height as f32,
                on,
                el().size(ICON_SIZE, ICON_SIZE)
                    .radius(4.0)
                    .fill(t.text_secondary.alpha(0.25)),
                text(["dupin", "dupin-core", "dupin-mac"][i])
                    .font(t.font(TextStyle::new(15.0, 500.0).lh(20.0)))
                    .color(if on { t.selected_text } else { t.text })
                    .truncate(),
                "Repository · → Actions",
            )
        })));
    el().size((w * scale).round(), (h * scale).round())
        .a11y_hidden()
        .child(panel.scale(scale).pin(
            Some((w * scale - w) / 2.0),
            Some((h * scale - h) / 2.0),
            None,
            None,
        ))
}

#[cfg(test)]
mod theme_tests {
    use super::*;
    #[test]
    fn duplicate_application_names_include_their_locations() {
        let home = std::env::var("HOME").unwrap_or_default();
        let user_applications = std::path::Path::new(&home).join("Applications");
        let apps = vec![
            dupin_mac::workspace::Application {
                path: "/Applications/Viewer.app".into(),
                name: "Viewer".into(),
                default: false,
            },
            dupin_mac::workspace::Application {
                path: user_applications.join("Viewer.app"),
                name: "viewer".into(),
                default: false,
            },
            dupin_mac::workspace::Application {
                path: "/Applications/Other.app".into(),
                name: "Other".into(),
                default: true,
            },
        ];
        assert_eq!(
            application_locations(&apps),
            vec![
                Some("/Applications".into()),
                Some(if home.is_empty() {
                    "Applications".into()
                } else {
                    "~/Applications".into()
                }),
                None,
            ]
        );
    }

    #[test]
    fn miniature_fits_both_axes_for_custom_geometry() {
        for width in [400, 680, 900, 1200] {
            for height in [52, 72, 100] {
                let theme = dupin_core::config::Theme {
                    panel_width: width,
                    row_height: height,
                    ..Default::default()
                };
                let (w, h, scale, n) = preview_geometry(&theme, 414.0);
                assert!(w * scale <= 414.01);
                assert!(h * scale <= 104.01);
                assert!(scale <= 0.5);
                assert_eq!(n, if height <= 72 { 3 } else { 2 });
            }
        }
    }
    #[test]
    fn themed_lists_keep_the_original_height_budget() {
        assert_eq!(visible_rows(52.0), 8);
        assert_eq!(visible_rows(44.0), 8);
        for height in [52.0, 64.0, 72.0, 92.0, 100.0] {
            assert!(visible_rows(height) as f32 * height <= LIST_HEIGHT_BUDGET);
        }
    }
    #[test]
    fn defaults_and_mode_overrides_resolve_independently() {
        let mut theme = dupin_core::config::Theme::default();
        assert_eq!(ResolvedTheme::new(&theme, false).text, tok::theme(false).text);
        assert_eq!(ResolvedTheme::new(&theme, true).card, tok::theme(true).card);
        theme.light.text = Some("#123456".into());
        assert_ne!(ResolvedTheme::new(&theme, false).text, tok::theme(false).text);
        assert_eq!(ResolvedTheme::new(&theme, true).text, tok::theme(true).text);
        theme.font_size = 18;
        theme.font_family = dupin_core::config::FontFamily::Monospace;
        let font = ResolvedTheme::new(&theme, false).font(TextStyle::new(15.0, 400.0).lh(20.0));
        assert_eq!(font.size, 18.0);
        assert_eq!(font.line_height, 24.0);
        assert_eq!(font.family, tok::Family::SystemMono);
    }
}

#[cfg(test)]
mod picker_tests {
    use super::*;

    #[test]
    #[ignore = "offscreen picker layout, run explicitly"]
    fn picker_rows_have_space_for_every_visible_value() {
        struct Preview {
            values: Vec<serde_json::Value>,
            edit: rune::TextEdit,
        }
        let app = rune::App::new(rune::AppSpec::new(
            "Picker layout",
            (680.0, 168.0),
            || Preview {
                values: vec![serde_json::json!("red"), serde_json::json!("blue")],
                edit: rune::TextEdit::new(""),
            },
            |s: &Preview, _| {
                picker_view(
                    text_input(&s.edit, TextStyle::new(22.0, 400.0)),
                    &s.values,
                    "",
                    0,
                    52.0,
                )
            },
        ));
        let root = std::env::temp_dir().join("dupin165-picker-layout");
        let tree = root.with_extension("json");
        let image = root.with_extension("png");
        rune::tooling::run_windows_with_args(
            Default::default(),
            [
                "render",
                "--tree",
                tree.to_str().unwrap(),
                "--snap",
                image.to_str().unwrap(),
                "--size",
                "680x168",
            ]
            .map(str::to_owned),
            move |_| [rune::WindowSpec::from_app("picker", app)],
        );
        let tree: serde_json::Value = serde_json::from_slice(&std::fs::read(tree).unwrap()).unwrap();
        let elements = tree["elements"].as_array().unwrap();
        for (index, y) in [(0, 64.0), (1, 116.0)] {
            let row = elements
                .iter()
                .find(|element| element["key"] == format!("pick-item-{index}"))
                .unwrap();
            assert_eq!(row["rect"][1].as_f64().unwrap(), y);
            assert_eq!(row["rect"][3].as_f64().unwrap(), 52.0);
            assert!(row["rect"][1].as_f64().unwrap() + row["rect"][3].as_f64().unwrap() <= 168.0);
        }
        println!("{}", image.display());
    }
}
