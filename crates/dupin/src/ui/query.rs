use dupin_core::{CommandId, Session};
use rune::{TextEdit, Tx};

use super::Ui;

impl Ui {
    /// Cursor-only redraws do not emit Rune input-change callbacks.
    pub fn query_display(&self) -> TextEdit {
        let mut query = self.query.clone();
        mark_session_query(&mut query, &self.session);
        query
    }

    pub fn query_edit(&mut self) -> &mut TextEdit {
        mark_session_query(&mut self.query, &self.session);
        &mut self.query
    }

    /// The field's text changed.
    pub fn edited(&mut self, tx: &mut Tx) {
        self.script_card = None;
        self.script_entry = None;
        if let Some(picker) = &mut self.picker {
            picker.selected = 0;
            tx.instant();
            return;
        }
        self.action_screen = None;
        self.typed_at = tx.clock();
        crate::latency::edit_started(self.typed_at, &self.query.text);
        let effects = self.session.set_query(&self.query.text);
        if !self.session.command().private_results {
            crate::trace::edited(&self.query.text);
        }
        self.apply(effects, tx);
        crate::latency::edit_applied(
            self.session.command_id(),
            !matches!(
                self.session.results(),
                dupin_core::session::Results::Searching { .. }
            ),
        );
    }
}

fn mark_query_qualifiers(edit: &mut TextEdit, command: &CommandId) {
    let marks: Vec<_> = crate::features::files::query_qualifier_ranges(command, &edit.text)
        .into_iter()
        .filter(|range| {
            ![edit.anchor, edit.focus]
                .into_iter()
                .any(|position| range.start < position && position < range.end)
        })
        .map(|range| rune::MarkRange::new(range, rune::Mark::Pill("file-qualifier".into())))
        .collect();
    if edit.marks() != marks.as_slice() {
        edit.set_marks(marks);
    }
}

/// A proposed file name is plain text: qualifiers only mean something in a search.
pub(super) fn mark_session_query(edit: &mut TextEdit, session: &Session) {
    if session.renaming().is_none() {
        mark_query_qualifiers(edit, session.command_id());
    } else if !edit.marks().is_empty() {
        edit.set_marks(Vec::new());
    }
}

#[cfg(test)]
mod qualifier_tests {
    use super::*;
    use rune::{Key, KeyEvent};

    fn files() -> CommandId {
        CommandId::new(crate::features::files::FILES).unwrap()
    }

    #[test]
    fn inserting_a_qualifier_before_text_keeps_the_caret_inside_the_unfinished_token() {
        let mut edit = TextEdit::new("report");
        edit.set_caret(0);
        edit.insert("in:/");
        mark_query_qualifiers(&mut edit, &files());
        assert_eq!(edit.text, "in:/report");
        assert_eq!(edit.focus, 4);
        assert_eq!(edit.pills().count(), 0);
        edit.insert("tmp ");
        mark_query_qualifiers(&mut edit, &files());
        assert_eq!(edit.text, "in:/tmp report");
        assert_eq!(edit.focus, 8);
        assert_eq!(edit.pills().count(), 1);
    }

    #[test]
    fn moving_the_caret_outside_a_token_allows_idempotent_marking() {
        let mut edit = TextEdit::new("in:/tmp report");
        edit.set_caret(4);
        mark_query_qualifiers(&mut edit, &files());
        assert_eq!(edit.focus, 4);
        assert_eq!(edit.pills().count(), 0);
        edit.set_caret(edit.text.len());
        mark_query_qualifiers(&mut edit, &files());
        assert_eq!(edit.pills().count(), 1);
        let before = edit.focus;
        mark_query_qualifiers(&mut edit, &files());
        assert_eq!(edit.focus, before);
        assert_eq!(edit.text, "in:/tmp report");
    }

    #[test]
    fn qualifier_pills_keep_raw_unicode_text_and_delete_as_whole_tokens() {
        let raw = "日本語 kind:pdf";
        let mut edit = TextEdit::new(raw);
        mark_query_qualifiers(&mut edit, &files());
        assert_eq!(edit.text, raw);
        let (range, _) = edit.pills().next().unwrap();
        assert_eq!(&edit.text[range], "kind:pdf");
        edit.set_caret(edit.text.len());
        edit.key(
            &KeyEvent {
                key: Key::Backspace,
                mods: Default::default(),
            },
            None,
        );
        mark_query_qualifiers(&mut edit, &files());
        assert_eq!(edit.text, "日本語 ");
        assert_eq!(edit.pills().count(), 0);
        edit.undo();
        mark_query_qualifiers(&mut edit, &files());
        assert_eq!(edit.text, raw);
        assert_eq!(edit.pills().count(), 1);
    }

    #[test]
    fn query_replacement_and_mode_changes_recompute_marks_without_changing_text() {
        let mut edit = TextEdit::new("kind:pdf");
        mark_query_qualifiers(&mut edit, &files());
        assert_eq!(edit.pills().count(), 1);
        edit.replace_text("report ext:rs");
        mark_query_qualifiers(&mut edit, &files());
        assert_eq!(edit.pills().count(), 1);
        let (range, _) = edit.pills().next().unwrap();
        assert_eq!(&edit.text[range], "ext:rs");
        mark_query_qualifiers(&mut edit, &CommandId::new(crate::features::apps::APPS).unwrap());
        assert_eq!(edit.text, "report ext:rs");
        assert_eq!(edit.pills().count(), 0);
    }
}
