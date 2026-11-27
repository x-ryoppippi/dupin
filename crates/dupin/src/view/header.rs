use dupin_core::session::Results;
use rune::tok::type_;
use rune::{Color, El, TextStyle, el, row, text, text_input};

use crate::ui::Ui;

use super::extensions::page_chip;
use super::theme::ResolvedTheme;

pub const HEADER_H: f32 = 64.0;

pub(super) fn header(s: &Ui, t: &ResolvedTheme, accent: Color) -> El {
    let presentation = &s.session.command().presentation;
    let renaming = s.session.renaming().is_some();
    let destination = s.session.destination();
    let page = s.session.page();
    let placeholder = if renaming {
        "Name"
    } else if destination.is_some() {
        "Search folders"
    } else if page.is_some() {
        "Filter"
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
    // A form takes what is typed into its fields, so the field shows the form's title.
    let field = match s.form_draft() {
        Some(draft) => text(&draft.form.title)
            .key("form-title")
            .grow(1.0)
            .min_w(0.0)
            .font(t.font(TextStyle::new(22.0, 400.0).lh(30.0)))
            .color(t.text)
            .truncate(),
        None => field,
    };
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
    let folder = s
        .session
        .folder()
        .map(|folder| folder.file_name().unwrap_or(folder.as_os_str()).to_string_lossy());
    let page_chip = page.map(|page| page_chip(page, presentation.chip.as_deref()));
    // The command's icon shows only beside the command's own name.
    let (chip, icon) = if let Some(chip) = destination_chip.as_deref() {
        (Some(chip), None)
    } else if renaming {
        (Some("Rename"), None)
    } else if let Some(folder) = folder.as_deref() {
        (Some(folder), None)
    } else {
        let chip = page_chip.as_deref().or(presentation.chip.as_deref());
        (chip, chip.and(s.session.command().icon()))
    };
    bar = if let Some(chip) = chip {
        let icon = icon
            .and_then(|icon| crate::symbols::icon(icon, crate::symbols::INLINE, accent))
            .map(|pixels| {
                el().key("mode-icon")
                    .size(crate::symbols::INLINE.square, crate::symbols::INLINE.square)
                    .pixels(&pixels)
            });
        bar.child(
            row()
                .key("mode")
                .max_w(280.0)
                .h(28.0)
                .px(10.0)
                .gap(6.0)
                .radius(7.0)
                .center_y()
                .fill(accent.alpha(0.16))
                .children(icon)
                .child(text(chip).font(t.font(type_::TITLE)).color(accent).truncate()),
        )
    } else {
        bar.child(t.search_icon())
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

pub(super) fn hint_row(s: &Ui, t: &ResolvedTheme) -> Option<El> {
    let hint = hint(s)?;
    Some(
        row()
            .key("hint")
            .w_full()
            .h(44.0)
            .px(20.0)
            .center_y()
            .child(text(hint).font(t.font(type_::BODY)).color(t.text_secondary)),
    )
}

pub(super) fn query_problem(s: &Ui) -> Option<&'static str> {
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
    crate::features::files::query_problem(s.session.command_id(), query)
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
    let empty =
        s.session.query().trim().is_empty() && s.session.folder().is_none() && s.session.page().is_none();
    match s.session.results() {
        Results::Idle if empty => presentation.empty_hint.as_deref(),
        Results::Done if s.session.failures().next().is_none() => {
            if let Some(text) = s.session.empty_text() {
                Some(text)
            } else if empty {
                presentation.empty_hint.as_deref()
            } else {
                presentation.no_results_hint.as_deref()
            }
        }
        _ => None,
    }
}
