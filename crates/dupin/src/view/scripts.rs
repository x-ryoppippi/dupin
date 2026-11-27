use dupin_core::script::split_args;
use rune::tok::type_;
use rune::{El, ScrollSpec, TextStyle, col, row, text, text_input};

use crate::scripts::{
    entry::Entry,
    runner::{Snapshot, Status},
};
use crate::ui::Ui;

use super::header::HEADER_H;
use super::results::LIST_HEIGHT_BUDGET;
use super::theme::{ResolvedTheme, divider};

pub(super) fn script_arguments(s: &Ui, entry: &Entry, t: &ResolvedTheme) -> [El; 3] {
    let header = &entry.script.header;
    let args = if s.script_entry_current(entry) {
        split_args(&entry.query.text)
    } else {
        Err("The script list changed. Press Escape to go back.".into())
    };
    let mut bindings = col().w_full().gap(8.0);
    let mut pending = false;
    for (arg, values) in header.argument_values(args.as_deref().unwrap_or_default()) {
        let mut label = col().w(144.0).gap(2.0).child(
            text(arg.name())
                .font(t.font(type_::TITLE))
                .color(t.text)
                .truncate(),
        );
        let kind = match (arg.is_optional(), arg.is_variadic()) {
            (true, true) => Some("Optional · Multiple values"),
            (true, false) => Some("Optional"),
            (false, true) => Some("One or more values"),
            (false, false) => None,
        };
        if let Some(kind) = kind {
            label = label.child(text(kind).font(t.font(type_::LABEL)).color(t.text_secondary));
        }
        let mut value = col().grow(1.0).min_w(0.0).gap(2.0);
        if values.is_empty() {
            let status = if args.is_err() {
                "—"
            } else if arg.is_optional() {
                "Omitted"
            } else {
                pending = true;
                "Waiting for input"
            };
            value = value.child(text(status).font(t.font(type_::BODY)).color(t.text_secondary));
        } else {
            value = value.children(values.iter().map(|value| {
                let (value, color) = if value.is_empty() {
                    ("Empty string", t.text_secondary)
                } else {
                    (value.as_str(), t.text)
                };
                text(value).w_full().font(t.font(type_::BODY)).color(color).wrap()
            }));
        }
        bindings = bindings.child(row().w_full().gap(12.0).child(label).child(value));
    }

    let problem = args.and_then(|args| header.bind_args(&args)).err();
    let mut summary = row().w_full().gap(12.0).center_y().child(
        text(&header.title)
            .grow(1.0)
            .min_w(0.0)
            .font(t.font(type_::TITLE))
            .color(t.text)
            .truncate(),
    );
    if problem.is_none() {
        summary = summary.child(
            text("↵ Run script")
                .font(t.font(type_::LABEL))
                .color(t.text_secondary),
        );
    }
    let mut preview = col()
        .key("argument-preview")
        .w_full()
        .px(18.0)
        .py(12.0)
        .gap(10.0)
        .child(summary);
    if let Some(problem) = problem.filter(|_| !pending) {
        preview = preview.child(
            text(problem)
                .key("arguments-problem")
                .w_full()
                .font(t.font(type_::BODY))
                .color(t.text)
                .wrap(),
        );
    }
    [
        row().w_full().h(HEADER_H).px(18.0).center_y().child(
            text_input(&entry.query, t.font(TextStyle::new(22.0, 400.0).lh(30.0)))
                .key("script-arguments")
                .label("Script arguments")
                .placeholder("Script arguments", t.text_secondary)
                .single_line()
                .grow(1.0)
                .color(t.text)
                .on_edit(|s: &mut Ui| &mut s.script_entry.as_mut().unwrap().query)
                .on_input_key(Ui::key),
        ),
        divider(t),
        preview.child(
            col()
                .w_full()
                .shrink()
                .max_h(LIST_HEIGHT_BUDGET - 60.0)
                .scroll(ScrollSpec { stick_bottom: false })
                .child(bindings),
        ),
    ]
}

pub(super) fn script_inline(run: &Snapshot, t: &ResolvedTheme) -> El {
    let output = run.output.last_stdout();
    let shortcut = match run.status {
        Status::Running => "⌘. Cancel · ⌘O Full output",
        Status::Exited(Some(0)) if output.is_some() => "↵ Copy · ⌘O Full output",
        Status::Exited(Some(0)) => "↵ Close · ⌘O Full output",
        _ => "⌘O Full output",
    };
    let mut card = col()
        .key("script-inline")
        .w_full()
        .px(18.0)
        .py(12.0)
        .gap(8.0)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .child(
                    text(&run.command.title)
                        .grow(1.0)
                        .min_w(0.0)
                        .font(t.font(type_::TITLE))
                        .color(t.text)
                        .truncate(),
                )
                .child(text(shortcut).font(t.font(type_::LABEL)).color(t.text_secondary)),
        );
    if let Some(output) = output {
        card = card.child(script_excerpt(output, t));
    }
    card = card.child(
        text(run.summary())
            .w_full()
            .font(t.font(type_::LABEL))
            .color(t.text_secondary)
            .wrap(),
    );
    if run.status.failed() {
        let stderr = run.output.stderr_tail();
        if !stderr.is_empty() {
            card = card.child(script_excerpt(stderr, t));
        }
    }
    card
}

fn script_excerpt(body: &str, t: &ResolvedTheme) -> El {
    col()
        .w_full()
        .shrink()
        .max_h(160.0)
        .scroll(ScrollSpec { stick_bottom: false })
        .child(text(body).w_full().font(t.font(type_::BODY)).color(t.text).wrap())
}
