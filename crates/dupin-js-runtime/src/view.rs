//! The pages a package's `views` answer with, after the reader has checked them, and the same
//! checks again for pages that crossed from the extension host.

use std::collections::HashSet;

use dupin_plugin_protocol::MAX_RESPONSE_BYTES;
use serde::{Deserialize, Serialize};

use crate::MAX_ROWS;
use crate::row::{
    MAX_TITLE_BYTES, Row, RowAction, RowDetail, Source, actions_text_bytes, check_handler, validate_actions,
};

/// Fields one form may have.
pub const MAX_FIELDS: usize = 16;
pub(crate) const MAX_FIELD_ID_BYTES: usize = 64;
pub(crate) const MAX_FIELD_TITLE_BYTES: usize = 128;
/// What a text field holds, its default or what is typed.
pub const MAX_TEXT_FIELD_BYTES: usize = 1024;
/// What a multi-line text field holds.
pub const MAX_TEXTAREA_BYTES: usize = 16 * 1024;
pub(crate) const MAX_OPTIONS: usize = 32;
pub(crate) const MAX_OPTION_BYTES: usize = 128;
pub(crate) const MAX_EMPTY_TEXT_BYTES: usize = 256;
/// One field's error, as a form's submit or Dupin's own check gives it.
pub const MAX_FIELD_ERROR_BYTES: usize = 256;

/// What `views[name](ctx)` answered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ViewAnswer {
    List {
        rows: Vec<Row>,
        /// What the page shows when it has no rows.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        empty_text: Option<String>,
    },
    Detail {
        title: String,
        detail: RowDetail,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        actions: Vec<RowAction>,
    },
    Form(Form),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Form {
    pub title: String,
    pub fields: Vec<Field>,
    /// The key of the package's `actions` that runs on submit, with the fields' values.
    pub submit: String,
    /// What the submit button says; Dupin says Submit without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submit_title: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub id: String,
    pub title: String,
    pub kind: FieldKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FieldKind {
    /// One line.
    Text {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        required: bool,
        #[serde(default)]
        default: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
    },
    /// Several lines.
    Textarea {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        required: bool,
        #[serde(default)]
        default: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
    },
    Checkbox {
        #[serde(default)]
        default: bool,
    },
    /// One of `options`, the first unless `default` names another.
    Select {
        options: Vec<SelectOption>,
        default: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectOption {
    pub value: String,
    pub title: String,
}

impl FieldKind {
    /// The most a text field of this kind may hold.
    pub fn max_text_bytes(&self) -> Option<usize> {
        match self {
            FieldKind::Text { .. } => Some(MAX_TEXT_FIELD_BYTES),
            FieldKind::Textarea { .. } => Some(MAX_TEXTAREA_BYTES),
            FieldKind::Checkbox { .. } | FieldKind::Select { .. } => None,
        }
    }

    pub fn required(&self) -> bool {
        matches!(
            self,
            FieldKind::Text { required: true, .. } | FieldKind::Textarea { required: true, .. }
        )
    }
}

fn line(name: &str, value: &str, cap: usize) -> Result<(), String> {
    if value.is_empty() || value.len() > cap || value.chars().any(char::is_control) {
        Err(format!("`{name}` must be one line of 1 to {cap} bytes"))
    } else {
        Ok(())
    }
}

/// A field's or option's ID: letters, digits, `_` and `-`.
pub(crate) fn check_field_id(id: &str) -> Result<(), String> {
    let valid = !id.is_empty()
        && id.len() <= MAX_FIELD_ID_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
    if valid {
        Ok(())
    } else {
        Err(format!(
            "must be 1 to {MAX_FIELD_ID_BYTES} letters, digits, `_` or `-`"
        ))
    }
}

impl ViewAnswer {
    /// Checks every limit the reader enforced, for a page from another process.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            ViewAnswer::List { rows, empty_text } => {
                if rows.len() > MAX_ROWS {
                    return Err(format!("more than {MAX_ROWS} rows"));
                }
                if let Some(text) = empty_text {
                    line("emptyText", text, MAX_EMPTY_TEXT_BYTES)?;
                }
                let mut ids = HashSet::new();
                for row in rows {
                    row.validate(Source::Search)?;
                    if !ids.insert(row.id.as_str()) {
                        return Err(format!("`{}` is the id of more than one row", row.id));
                    }
                }
                let bytes: usize = rows.iter().map(Row::text_bytes).sum();
                if bytes > MAX_RESPONSE_BYTES {
                    return Err("more text than a page may carry".into());
                }
                Ok(())
            }
            ViewAnswer::Detail {
                title,
                detail,
                actions,
            } => {
                line("title", title, MAX_TITLE_BYTES)?;
                detail.validate()?;
                validate_actions(actions, Source::Search)?;
                if title.len() + detail.text_bytes() + actions_text_bytes(actions) > MAX_RESPONSE_BYTES {
                    return Err("more text than a page may carry".into());
                }
                Ok(())
            }
            ViewAnswer::Form(form) => form.validate(),
        }
    }
}

impl Form {
    pub fn validate(&self) -> Result<(), String> {
        line("title", &self.title, MAX_TITLE_BYTES)?;
        check_handler(&self.submit).map_err(|message| message.replace("`run`", "`submit`"))?;
        if let Some(title) = &self.submit_title {
            line("submitTitle", title, MAX_FIELD_TITLE_BYTES)?;
        }
        if self.fields.is_empty() || self.fields.len() > MAX_FIELDS {
            return Err(format!("a form has 1 to {MAX_FIELDS} fields"));
        }
        let mut ids = HashSet::new();
        for field in &self.fields {
            check_field_id(&field.id).map_err(|message| format!("a field's `id` {message}"))?;
            if !ids.insert(field.id.as_str()) {
                return Err(format!("`{}` is the id of more than one field", field.id));
            }
            line("fields.title", &field.title, MAX_FIELD_TITLE_BYTES)?;
            match &field.kind {
                FieldKind::Text {
                    default, placeholder, ..
                }
                | FieldKind::Textarea {
                    default, placeholder, ..
                } => {
                    let max = field.kind.max_text_bytes().expect("a text field");
                    if default.len() > max {
                        return Err(format!("`{}`'s default is longer than {max} bytes", field.id));
                    }
                    if matches!(field.kind, FieldKind::Text { .. }) && default.chars().any(char::is_control) {
                        return Err(format!("`{}`'s default must be one line", field.id));
                    }
                    if let Some(placeholder) = placeholder {
                        line("fields.placeholder", placeholder, MAX_FIELD_TITLE_BYTES)?;
                    }
                }
                FieldKind::Checkbox { .. } => {}
                FieldKind::Select { options, default } => {
                    if options.is_empty() || options.len() > MAX_OPTIONS {
                        return Err(format!("a select has 1 to {MAX_OPTIONS} options"));
                    }
                    let mut values = HashSet::new();
                    for option in options {
                        line("options.value", &option.value, MAX_OPTION_BYTES)?;
                        line("options.title", &option.title, MAX_OPTION_BYTES)?;
                        if !values.insert(option.value.as_str()) {
                            return Err(format!("`{}` is the value of more than one option", option.value));
                        }
                    }
                    if !values.contains(default.as_str()) {
                        return Err(format!("`{}`'s default isn't one of its options", field.id));
                    }
                }
            }
        }
        Ok(())
    }
}
