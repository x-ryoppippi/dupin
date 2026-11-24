use dupin_core::View;
use rune::tok::type_;
use rune::{Color, El, ScrollSpec, col, row, text};

use crate::ui::{ActionStage, ApplicationState, SEND_TO, Ui};

use super::header::HEADER_H;
use super::results::{LIST_HEIGHT_BUDGET, LIST_PAD, item_icon, visible_rows};
use super::theme::ResolvedTheme;

pub(super) fn action_header(s: &Ui, t: &ResolvedTheme) -> El {
    let screen = s.action_screen.as_ref().unwrap();
    let icon = item_icon(s, screen.item.icon.as_ref(), false);
    let subtitle = match (&screen.stage, &screen.item.view) {
        (ActionStage::Receivers { .. }, _) => SEND_TO,
        (_, View::Text { subtitle, .. }) if !subtitle.is_empty() => subtitle.as_str(),
        _ => "Actions",
    };
    row()
        .key("action-context")
        .w_full()
        .h(HEADER_H)
        .px(18.0)
        .gap(12.0)
        .center_y()
        .children(icon)
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

fn application_locations(apps: &[crate::platform::workspace::Application]) -> Vec<Option<String>> {
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

pub(super) fn action_list(s: &Ui, t: &ResolvedTheme, accent: Color) -> El {
    let screen = s.action_screen.as_ref().unwrap();
    let row_height = s.theme.row_height as f32 - 8.0;
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
                                let icon =
                                    item_icon(s, Some(&dupin_core::Icon::File(app.path.clone())), true);
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
                                    .children(icon)
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
        ActionStage::Receivers { actions, selected } => {
            return action_options("receivers", row_height).children(actions.iter().enumerate().map(
                |(index, action)| {
                    action_row(&action.title, index, *selected, row_height, false, t, accent)
                        .on_press_tx(move |s: &mut Ui, tx| s.pick_receiver(index, tx))
                },
            ));
        }
        ActionStage::Confirm {
            action_id, selected, ..
        } => {
            let action = screen.item.item_action(action_id).unwrap();
            let confirmation = action.confirmation().unwrap();
            let path = match screen.item.action(action_id) {
                Some(dupin_core::Action::Trash(path)) => Some(path.to_string_lossy()),
                _ => None,
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
                        .children(path.as_deref().map(|path| {
                            text(path)
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
    action_options("actions", row_height)
        .children(screen.item.actions.iter().enumerate().map(|(index, action)| {
            let key = screen.key.clone();
            let id = action.id.clone();
            action_row(
                &action.title,
                index,
                screen.selected,
                row_height,
                action.confirmation().is_some(),
                t,
                accent,
            )
            .on_press_tx(move |s: &mut Ui, tx| s.pick_action(&key, &id, tx))
        }))
        .children((!screen.item.payload.is_empty()).then(|| {
            action_row(
                SEND_TO,
                screen.item.actions.len(),
                screen.selected,
                row_height,
                false,
                t,
                accent,
            )
            .on_press_tx(Ui::show_receivers)
        }))
}

fn action_options(key: &str, row_height: f32) -> El {
    col()
        .key(key)
        .w_full()
        .py(LIST_PAD)
        .max_h(row_height * visible_rows(row_height) as f32 + 2.0 * LIST_PAD)
        .scroll(ScrollSpec { stick_bottom: false })
}

fn action_row(
    title: &str,
    index: usize,
    selected: usize,
    height: f32,
    destructive: bool,
    t: &ResolvedTheme,
    accent: Color,
) -> El {
    let on = index == selected;
    row()
        .key(format!("action-{index}"))
        .w_full()
        .h(height)
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
                    text(title)
                        .font(t.font(type_::BODY))
                        .color(if on {
                            t.selected_text
                        } else if destructive {
                            t.destructive
                        } else {
                            t.text
                        })
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                )
                .child(
                    text(if on { "↩" } else { "" })
                        .font(t.font(type_::LABEL))
                        .color(t.selected_text.alpha(0.78)),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::application_locations;

    #[test]
    fn duplicate_application_names_include_their_locations() {
        let home = std::env::var("HOME").unwrap_or_default();
        let user_applications = std::path::Path::new(&home).join("Applications");
        let apps = vec![
            crate::platform::workspace::Application {
                path: "/Applications/Viewer.app".into(),
                name: "Viewer".into(),
                default: false,
            },
            crate::platform::workspace::Application {
                path: user_applications.join("Viewer.app"),
                name: "viewer".into(),
                default: false,
            },
            crate::platform::workspace::Application {
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
}
