//! What is typed into an extension's form. Dupin keeps a page's drafts while the page stays
//! open, including while a page above it is, moves focus between its fields, checks that
//! required fields are filled in and hands the package the values only on submit.

use std::collections::BTreeMap;
use std::sync::Arc;

use dupin_core::form::{FieldKind, Form, Value};
use dupin_core::{Intent, Navigated};
use rune::{Key, KeyEvent, TextEdit, Tx};

use super::Ui;

/// The drafts of the form on one page of the stack.
pub(crate) struct Draft {
    /// How many pages deep the form is, which drafts stay with while pages above it open.
    depth: usize,
    pub form: Arc<Form>,
    pub entries: Vec<Entry>,
    pub focus: usize,
    /// What is wrong with each field, by field ID, as Dupin or the package said.
    pub errors: BTreeMap<String, String>,
    /// The package is taking the values, so another submit waits for it.
    pub submitting: bool,
}

pub(crate) enum Entry {
    Text(Box<TextEdit>),
    Checkbox(bool),
    Select(usize),
}

impl Entry {
    fn new(value: Value) -> Self {
        match value {
            Value::Text(text) => Self::Text(Box::new(TextEdit::new(&text))),
            Value::Checkbox(on) => Self::Checkbox(on),
            Value::Select(index) => Self::Select(index),
        }
    }

    fn value(&self) -> Value {
        match self {
            Entry::Text(edit) => Value::Text(edit.text.clone()),
            Entry::Checkbox(on) => Value::Checkbox(*on),
            Entry::Select(index) => Value::Select(*index),
        }
    }
}

/// The element key of field `index`'s control.
pub(crate) fn field_key(index: usize) -> String {
    format!("form-field-{index}")
}

fn field_of_key(key: &str) -> Option<usize> {
    key.rsplit('/').next()?.strip_prefix("form-field-")?.parse().ok()
}

impl Draft {
    fn new(depth: usize, form: Arc<Form>) -> Self {
        let entries = form
            .fields
            .iter()
            .map(|field| Entry::new(field.default_value()))
            .collect();
        Self {
            depth,
            form,
            entries,
            focus: 0,
            errors: BTreeMap::new(),
            submitting: false,
        }
    }

    fn values(&self) -> Vec<Value> {
        self.entries.iter().map(Entry::value).collect()
    }

    /// Takes `form`, a new answer for the same page, keeping what was typed while its fields
    /// are the same.
    fn renew(&mut self, form: Arc<Form>) {
        let same_fields = form.fields.len() == self.form.fields.len()
            && form
                .fields
                .iter()
                .zip(&self.form.fields)
                .all(|(new, old)| new.id == old.id)
            && form.fits(&self.values());
        if same_fields {
            self.form = form;
        } else {
            *self = Self::new(self.depth, form);
        }
    }

    fn is_text(&self, index: usize) -> bool {
        matches!(self.entries.get(index), Some(Entry::Text(_)))
    }

    fn multiline(&self, index: usize) -> bool {
        matches!(
            self.form.fields.get(index).map(|field| &field.kind),
            Some(FieldKind::Text { multiline: true, .. })
        )
    }
}

impl Ui {
    /// The drafts of the form the page on screen shows.
    pub(crate) fn form_draft(&self) -> Option<&Draft> {
        let depth = self.session.depth();
        self.session.form()?;
        self.forms.iter().find(|draft| draft.depth == depth)
    }

    fn form_draft_mut(&mut self) -> Option<&mut Draft> {
        let depth = self.session.depth();
        self.session.form()?;
        self.forms.iter_mut().find(|draft| draft.depth == depth)
    }

    /// Keeps the drafts in step with the pages: drops those of closed pages, starts drafts for
    /// a form that just showed and keeps a page's drafts across its view's new answers.
    pub(super) fn sync_forms(&mut self, tx: &mut Tx) {
        let depth = self.session.depth();
        self.forms.retain(|draft| draft.depth <= depth);
        let Some(form) = self.session.form().cloned() else {
            // A page that answered with something else no longer holds a form; one that is
            // still being asked keeps its drafts for the answer.
            if self.session.results() == dupin_core::session::Results::Done {
                self.forms.retain(|draft| draft.depth != depth);
            }
            if std::mem::replace(&mut self.showing_form, false) {
                tx.focus("query", false);
            }
            return;
        };
        let shown_before = std::mem::replace(&mut self.showing_form, true);
        match self.forms.iter_mut().find(|draft| draft.depth == depth) {
            Some(draft) if Arc::ptr_eq(&draft.form, &form) => {}
            Some(draft) => draft.renew(form),
            None => {
                self.forms.push(Draft::new(depth, form));
                self.focus_field(0, tx);
                return;
            }
        }
        if !shown_before {
            let focus = self.form_draft().map_or(0, |draft| draft.focus);
            self.focus_field(focus, tx);
        }
    }

    /// The text field `index` edits, for the field's own edits.
    pub(crate) fn form_edit(&mut self, index: usize) -> &mut TextEdit {
        let edits = self.form_draft().is_some_and(|draft| draft.is_text(index));
        if !edits {
            return &mut self.form_scratch;
        }
        match self
            .form_draft_mut()
            .and_then(|draft| draft.entries.get_mut(index))
        {
            Some(Entry::Text(edit)) => edit,
            _ => unreachable!("checked above"),
        }
    }

    /// A field's text changed, which answers what was wrong with it.
    pub(crate) fn form_edited(&mut self, index: usize) {
        if let Some(draft) = self.form_draft_mut()
            && let Some(field) = draft.form.fields.get(index)
        {
            let id = field.id.clone();
            draft.errors.remove(&id);
        }
    }

    fn focus_field(&mut self, index: usize, tx: &mut Tx) {
        let Some(draft) = self.form_draft_mut() else {
            return;
        };
        if draft.entries.is_empty() {
            return;
        }
        let index = index.min(draft.entries.len() - 1);
        draft.focus = index;
        if draft.is_text(index) {
            tx.focus(field_key(index), true);
        } else {
            tx.blur();
        }
    }

    /// A press on field `index`: it takes focus, and a checkbox or select changes.
    pub(crate) fn press_field(&mut self, index: usize, tx: &mut Tx) {
        self.focus_field(index, tx);
        self.change_field(index, 1);
    }

    fn change_field(&mut self, index: usize, step: isize) -> bool {
        let Some(draft) = self.form_draft_mut() else {
            return false;
        };
        let options = match draft.form.fields.get(index).map(|field| &field.kind) {
            Some(FieldKind::Select { options, .. }) => options.len(),
            _ => 0,
        };
        let id = draft.form.fields.get(index).map(|field| field.id.clone());
        let changed = match draft.entries.get_mut(index) {
            Some(Entry::Checkbox(on)) => {
                *on = !*on;
                true
            }
            Some(Entry::Select(chosen)) if options > 0 => {
                *chosen = (*chosen as isize + step).rem_euclid(options as isize) as usize;
                true
            }
            _ => false,
        };
        if changed && let Some(id) = id {
            draft.errors.remove(&id);
        }
        changed
    }

    /// Keys while a form shows. True consumes the key.
    pub(crate) fn form_key(&mut self, ev: &KeyEvent, tx: &mut Tx) -> bool {
        let focused = tx.focused().and_then(field_of_key);
        let Some(draft) = self.form_draft_mut() else {
            return false;
        };
        if let Some(index) = focused {
            draft.focus = index;
        }
        let focus = draft.focus;
        if matches!(draft.entries.get(focus), Some(Entry::Text(edit)) if edit.is_composing()) {
            return false;
        }
        let (text, multiline) = (draft.is_text(focus), draft.multiline(focus));
        let count = draft.entries.len();
        let m = ev.mods;
        let plain = !(m.cmd || m.ctrl || m.alt);
        match &ev.key {
            Key::Escape => {
                if let Some(effects) = self.session.intent(Intent::Dismiss) {
                    self.apply(effects, tx);
                }
                true
            }
            Key::Tab if plain => {
                let next = if m.shift {
                    (focus + count - 1) % count
                } else {
                    (focus + 1) % count
                };
                self.focus_field(next, tx);
                true
            }
            Key::Enter if m.cmd => {
                self.submit_form(tx);
                true
            }
            Key::Enter if plain && multiline && !tx.key_repeat() => {
                self.form_edit(focus).insert("\n");
                self.form_edited(focus);
                true
            }
            Key::Enter if plain => {
                if !tx.key_repeat() {
                    self.submit_form(tx);
                }
                true
            }
            Key::Text(typed) if typed == " " && plain && !text => self.change_field(focus, 1),
            Key::Right if !text => self.change_field(focus, 1),
            Key::Left if !text => self.change_field(focus, -1),
            Key::Down if !multiline => {
                self.focus_field((focus + 1) % count, tx);
                true
            }
            Key::Up if !multiline => {
                self.focus_field((focus + count - 1) % count, tx);
                true
            }
            _ => false,
        }
    }

    /// Checks the drafts and, once every field is filled in as it must be, hands the values
    /// to the package's submit action, as choosing an action would.
    pub(crate) fn submit_form(&mut self, tx: &mut Tx) {
        let Some(draft) = self.form_draft_mut() else {
            return;
        };
        if draft.submitting {
            return;
        }
        let form = draft.form.clone();
        match form.submission(&draft.values()) {
            Err(errors) => {
                let first = form
                    .fields
                    .iter()
                    .position(|field| errors.contains_key(&field.id))
                    .unwrap_or(0);
                draft.errors = errors;
                self.focus_field(first, tx);
            }
            Ok(values) => {
                draft.errors.clear();
                draft.submitting = true;
                self.run_extension_action(
                    form.extension.clone(),
                    form.submit.clone(),
                    serde_json::Value::Object(values),
                );
            }
        }
    }

    /// The package answered a submit; another may follow.
    pub(super) fn form_answered(&mut self, anchor: &dupin_core::Anchor) {
        if self.session.is_current(anchor)
            && let Some(draft) = self.form_draft_mut()
        {
            draft.submitting = false;
        }
    }

    /// Shows the package's `errors` under the fields of the form `anchor` names, while it
    /// still shows. Errors for fields the form doesn't have are left out.
    pub(super) fn show_field_errors(
        &mut self,
        anchor: &dupin_core::Anchor,
        errors: BTreeMap<String, String>,
        tx: &mut Tx,
    ) -> Navigated {
        if !self.session.is_current(anchor) {
            return Navigated::Stale;
        }
        let Some(draft) = self.form_draft_mut() else {
            return Navigated::NoPage;
        };
        let known: BTreeMap<String, String> = errors
            .into_iter()
            .filter(|(id, _)| draft.form.fields.iter().any(|field| field.id == *id))
            .collect();
        let first = draft
            .form
            .fields
            .iter()
            .position(|field| known.contains_key(&field.id));
        draft.errors = known;
        if let Some(first) = first {
            self.focus_field(first, tx);
        }
        Navigated::Done(Vec::new())
    }
}
