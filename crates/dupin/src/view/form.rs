use dupin_core::form::FieldKind;
use rune::tok::type_;
use rune::{Color, El, ScrollSpec, TextStyle, auto_grow, col, edit_access, el, row, text, text_input};

use crate::ui::{Draft, Entry, Ui, field_key};

use super::results::{LIST_HEIGHT_BUDGET, LIST_PAD};
use super::theme::ResolvedTheme;

const LABEL_W: f32 = 144.0;
const CONTROL_H: f32 = 34.0;

/// An extension's form: each field beside its title, with what is wrong with it beneath, and
/// the keys that submit and cancel it.
pub(super) fn form(draft: &Draft, t: &ResolvedTheme, accent: Color) -> El {
    let fields = draft
        .form
        .fields
        .iter()
        .zip(&draft.entries)
        .enumerate()
        .map(|(index, (field, entry))| {
            let focused = index == draft.focus;
            let mut label = col().w(LABEL_W).pt(8.0).gap(2.0).child(
                text(&field.title)
                    .font(t.font(type_::TITLE))
                    .color(t.text)
                    .truncate(),
            );
            if field.required() {
                label = label.child(
                    text("Required")
                        .font(t.font(type_::LABEL))
                        .color(t.text_secondary),
                );
            }
            let mut control = col().grow(1.0).min_w(0.0).gap(4.0).child(field_control(
                index,
                &field.kind,
                entry,
                focused,
                t,
                accent,
            ));
            if let Some(error) = draft.errors.get(&field.id) {
                control = control.child(
                    text(error)
                        .key(format!("{}-error", field_key(index)))
                        .w_full()
                        .font(t.font(type_::LABEL))
                        .color(t.destructive)
                        .wrap(),
                );
            }
            row().w_full().gap(12.0).child(label).child(control)
        });
    let hint = if draft.submitting {
        "Submitting…".to_owned()
    } else {
        format!("⌘↵ {} · ⇥ Next field · esc Cancel", draft.form.submit_title)
    };
    col()
        .key("form")
        .w_full()
        .px(18.0)
        .py(12.0)
        .gap(12.0)
        .child(
            col()
                .key("form-scroll")
                .w_full()
                .shrink()
                .max_h(LIST_HEIGHT_BUDGET - 48.0)
                .scroll(ScrollSpec { stick_bottom: false })
                .child(col().w_full().py(LIST_PAD).gap(14.0).children(fields)),
        )
        .child(
            row().w_full().center_y().child(el().grow(1.0)).child(
                text(hint)
                    .key("form-hint")
                    .font(t.font(type_::LABEL))
                    .color(t.text_secondary),
            ),
        )
}

fn field_control(
    index: usize,
    kind: &FieldKind,
    entry: &Entry,
    focused: bool,
    t: &ResolvedTheme,
    accent: Color,
) -> El {
    let frame = |content: El| {
        row()
            .key(format!("{}-frame", field_key(index)))
            .w_full()
            .min_h(CONTROL_H)
            .px(10.0)
            .radius(7.0)
            .center_y()
            .fill(t.hover)
            .border(1.0, if focused { accent } else { Color::TRANSPARENT })
            .child(content)
    };
    match (kind, entry) {
        (
            FieldKind::Text {
                multiline,
                placeholder,
                ..
            },
            Entry::Text(edit),
        ) => {
            let mut field = text_input(edit, t.font(TextStyle::new(15.0, 400.0).lh(22.0)))
                .key(field_key(index))
                .grow(1.0)
                .min_w(0.0)
                .color(t.text)
                .on_edit(edit_access(move |s: &mut Ui| s.form_edit(index)))
                .on_input_key(|s: &mut Ui, ev, tx| s.form_key(ev, tx))
                .on_input_change(move |s: &mut Ui, _| s.form_edited(index));
            if let Some(placeholder) = placeholder {
                field = field
                    .label(placeholder.as_str())
                    .placeholder(placeholder.as_str(), t.text_secondary);
            }
            if *multiline {
                frame(col().w_full().py(6.0).child(auto_grow(field, 3, 8)))
            } else {
                frame(field.single_line())
            }
        }
        (FieldKind::Checkbox { .. }, Entry::Checkbox(on)) => frame(
            row()
                .key(field_key(index))
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    text(if *on { "☑" } else { "☐" })
                        .font(t.font(TextStyle::new(18.0, 400.0)))
                        .color(if *on { accent } else { t.text_secondary }),
                )
                .child(
                    text(if *on { "On" } else { "Off" })
                        .font(t.font(type_::BODY))
                        .color(t.text),
                ),
        )
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(move |s: &mut Ui, tx| s.press_field(index, tx)),
        (FieldKind::Select { options, .. }, Entry::Select(chosen)) => {
            let title = options.get(*chosen).map_or("", |(_, title)| title.as_str());
            frame(
                row()
                    .key(field_key(index))
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(
                        text(title)
                            .grow(1.0)
                            .min_w(0.0)
                            .font(t.font(type_::BODY))
                            .color(t.text)
                            .truncate(),
                    )
                    .child(
                        text(format!("{} of {}", chosen + 1, options.len()))
                            .font(t.font(type_::LABEL))
                            .color(t.text_secondary),
                    )
                    .child(text("‹ ›").font(t.font(type_::BODY)).color(t.text_secondary)),
            )
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(move |s: &mut Ui, tx| s.press_field(index, tx))
        }
        _ => el(),
    }
}
