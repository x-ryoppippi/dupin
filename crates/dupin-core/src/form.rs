//! A form an extension's page shows. Dupin owns what is typed into it: the drafts, the order
//! the fields take focus in and the check that required fields are filled in, and hands the
//! extension the values only when the user submits it.

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form {
    pub title: String,
    pub fields: Vec<Field>,
    /// What the submit button says.
    pub submit_title: String,
    /// The extension, by its ID as [`crate::Action::Extension`] names it, whose action takes
    /// the values.
    pub extension: String,
    /// The key of that extension's `actions` that takes the values.
    pub submit: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub id: String,
    pub title: String,
    pub kind: FieldKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Text {
        multiline: bool,
        required: bool,
        default: String,
        placeholder: Option<String>,
        /// The most it may hold, in bytes.
        max_bytes: usize,
    },
    Checkbox {
        default: bool,
    },
    Select {
        /// Each option's value and title.
        options: Vec<(String, String)>,
        /// The index of the option chosen at first.
        default: usize,
    },
}

/// What one field holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Text(String),
    Checkbox(bool),
    /// The index of the chosen option.
    Select(usize),
}

impl Field {
    pub fn default_value(&self) -> Value {
        match &self.kind {
            FieldKind::Text { default, .. } => Value::Text(default.clone()),
            FieldKind::Checkbox { default } => Value::Checkbox(*default),
            FieldKind::Select { default, .. } => Value::Select(*default),
        }
    }

    /// What is wrong with `value` for this field, as Dupin shows it under the field.
    fn problem(&self, value: &Value) -> Option<String> {
        match (&self.kind, value) {
            (
                FieldKind::Text {
                    multiline,
                    required,
                    max_bytes,
                    ..
                },
                Value::Text(text),
            ) => {
                if *required && text.trim().is_empty() {
                    Some("Required".into())
                } else if text.len() > *max_bytes {
                    Some(format!("Too long: at most {max_bytes} bytes"))
                } else if !multiline && text.chars().any(char::is_control) {
                    Some("One line only".into())
                } else {
                    None
                }
            }
            (FieldKind::Checkbox { .. }, Value::Checkbox(_)) => None,
            (FieldKind::Select { options, .. }, Value::Select(index)) if *index < options.len() => None,
            _ => Some("Choose a value".into()),
        }
    }

    pub fn required(&self) -> bool {
        matches!(self.kind, FieldKind::Text { required: true, .. })
    }
}

impl Form {
    /// The values as the extension's submit receives them, by field ID, or what is wrong with
    /// each field that isn't filled in as it must be. `values` holds one value per field, in
    /// the fields' order.
    pub fn submission(
        &self,
        values: &[Value],
    ) -> Result<serde_json::Map<String, serde_json::Value>, BTreeMap<String, String>> {
        let mut errors = BTreeMap::new();
        let mut submitted = serde_json::Map::new();
        for (index, field) in self.fields.iter().enumerate() {
            let value = values.get(index);
            let problem = match value {
                Some(value) => field.problem(value),
                None => Some("Choose a value".into()),
            };
            if let Some(problem) = problem {
                errors.insert(field.id.clone(), problem);
                continue;
            }
            let json = match (value.expect("checked above"), &field.kind) {
                (Value::Text(text), _) => serde_json::Value::String(text.clone()),
                (Value::Checkbox(on), _) => serde_json::Value::Bool(*on),
                (Value::Select(index), FieldKind::Select { options, .. }) => {
                    serde_json::Value::String(options[*index].0.clone())
                }
                (Value::Select(_), _) => unreachable!("checked above"),
            };
            submitted.insert(field.id.clone(), json);
        }
        if errors.is_empty() {
            Ok(submitted)
        } else {
            Err(errors)
        }
    }

    /// Whether `values` still fits the fields, so drafts made for an earlier answer can be kept.
    pub fn fits(&self, values: &[Value]) -> bool {
        values.len() == self.fields.len()
            && self.fields.iter().zip(values).all(|(field, value)| {
                matches!(
                    (&field.kind, value),
                    (FieldKind::Text { .. }, Value::Text(_))
                        | (FieldKind::Checkbox { .. }, Value::Checkbox(_))
                ) || matches!((&field.kind, value), (FieldKind::Select { options, .. }, Value::Select(index)) if *index < options.len())
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> Form {
        Form {
            title: "New note".into(),
            fields: vec![
                Field {
                    id: "title".into(),
                    title: "Title".into(),
                    kind: FieldKind::Text {
                        multiline: false,
                        required: true,
                        default: String::new(),
                        placeholder: None,
                        max_bytes: 8,
                    },
                },
                Field {
                    id: "body".into(),
                    title: "Body".into(),
                    kind: FieldKind::Text {
                        multiline: true,
                        required: false,
                        default: "a\nb".into(),
                        placeholder: None,
                        max_bytes: 64,
                    },
                },
                Field {
                    id: "pinned".into(),
                    title: "Pinned".into(),
                    kind: FieldKind::Checkbox { default: true },
                },
                Field {
                    id: "label".into(),
                    title: "Label".into(),
                    kind: FieldKind::Select {
                        options: vec![("home".into(), "Home".into()), ("work".into(), "Work".into())],
                        default: 1,
                    },
                },
            ],
            submit_title: "Save".into(),
            extension: "notes".into(),
            submit: "save".into(),
        }
    }

    #[test]
    fn a_submission_holds_every_value_by_field_id_once_required_fields_are_filled_in() {
        let form = form();
        let mut values: Vec<Value> = form.fields.iter().map(Field::default_value).collect();
        assert_eq!(
            form.submission(&values).unwrap_err(),
            BTreeMap::from([("title".to_owned(), "Required".to_owned())])
        );
        values[0] = Value::Text("   ".into());
        assert!(form.submission(&values).is_err(), "spaces are no title");
        values[0] = Value::Text("Tea".into());
        assert_eq!(
            serde_json::Value::Object(form.submission(&values).unwrap()),
            serde_json::json!({"title": "Tea", "body": "a\nb", "pinned": true, "label": "work"})
        );
    }

    #[test]
    fn drafts_that_outgrow_their_field_or_break_its_line_are_refused() {
        let form = form();
        let mut values: Vec<Value> = form.fields.iter().map(Field::default_value).collect();
        values[0] = Value::Text("a\nb".into());
        values[1] = Value::Text("x".repeat(65));
        values[3] = Value::Select(9);
        let errors = form.submission(&values).unwrap_err();
        assert_eq!(errors["title"], "One line only");
        assert_eq!(errors["body"], "Too long: at most 64 bytes");
        assert_eq!(errors["label"], "Choose a value");
        assert!(!form.fits(&values));
        assert!(form.fits(&form.fields.iter().map(Field::default_value).collect::<Vec<_>>()));
        assert!(!form.fits(&[Value::Checkbox(true)]));
    }
}
