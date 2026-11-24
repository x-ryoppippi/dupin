use crate::platform::{accessibility, selection::Selection};
use rune::{TextEdit, Tx};

use super::Ui;

#[derive(Default, PartialEq, Eq)]
pub(super) enum SelectionRequest {
    #[default]
    Idle,
    Reading,
    Superseded,
}

impl Ui {
    pub(super) fn read_selection(&mut self, tx: &mut Tx) {
        if rune::tooling::is_headless()
            || self.shown
            || self.picker.is_some()
            || self.selection_request != SelectionRequest::Idle
        {
            return;
        }
        self.selection_request = SelectionRequest::Reading;
        let panel = self.me.clone();
        if std::thread::Builder::new()
            .name("selected-text".into())
            .spawn(move || {
                let result = crate::platform::selection::read(|| {
                    accessibility::request_once();
                });
                panel.update(move |s, tx| s.apply_selection(result, tx));
            })
            .is_err()
        {
            self.apply_selection(Selection::Empty, tx);
        }
    }

    fn apply_selection(&mut self, result: Selection, tx: &mut Tx) {
        if std::mem::take(&mut self.selection_request) != SelectionRequest::Reading {
            return;
        }
        match result {
            Selection::Text(text) => {
                self.query = selection_query(&text);
                self.typed_at = tx.clock();
                let effects = self.session.set_literal_query(&self.query.text);
                self.apply(effects, tx);
            }
            Selection::Empty => {}
            Selection::PermissionRequired => {
                self.selection_permission_required = !accessibility::is_trusted();
            }
        }
        self.show(tx);
        tx.focus_when_shown("query", false);
    }
}

fn selection_query(text: &str) -> TextEdit {
    const MAX_QUERY_CHARS: usize = 500;
    let mut chars = text.trim().chars().peekable();
    let query: String = std::iter::from_fn(|| {
        let next = chars.next()?;
        if next == '\r' {
            chars.next_if_eq(&'\n');
        }
        Some(match next {
            '\r' | '\n' | '\t' | '\u{2028}' | '\u{2029}' => ' ',
            other => other,
        })
    })
    .take(MAX_QUERY_CHARS)
    .collect();
    let mut edit = TextEdit::new("");
    edit.set_single_line(true);
    edit.insert(query.trim_end());
    edit.set_selection(0, edit.text.len());
    edit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_replaces_the_entire_selected_query() {
        let mut edit = selection_query("\t 日本語\r\nsecond line\u{2029} ");
        assert_eq!(edit.text, "日本語 second line");
        assert_eq!(edit.range(), (0, edit.text.len()));
        edit.insert("calculator");
        assert_eq!(edit.text, "calculator");
    }

    #[test]
    fn a_document_selection_becomes_a_bounded_unicode_query() {
        let text = format!("{}\r\n続き", "猫".repeat(500));
        let edit = selection_query(&text);
        assert_eq!(edit.text, "猫".repeat(500));
        assert_eq!(edit.range(), (0, edit.text.len()));
    }
}
