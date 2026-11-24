//! Reads what `search` returned into protocol rows, checking every value against the
//! protocol's limits.

use std::collections::HashSet;

use dupin_plugin_protocol::{
    Effect, MAX_CONTEXT_BYTES, MAX_CONTEXT_DEPTH, MAX_RESPONSE_BYTES, PluginAction, PluginItem, View,
    is_web_url,
};
use rquickjs::{Object, Value};
use serde::{Deserialize, Serialize};

use crate::{Diagnostic, Failure, MAX_HANDLER_BYTES, MAX_ROWS};

const ITEM_FIELDS: [&str; 6] = ["id", "title", "subtitle", "text", "url", "actions"];
const ACTION_FIELDS: [&str; 3] = ["title", "run", "data"];
/// A row offers at most eight actions, Open URL and Copy included.
const MAX_ACTIONS: usize = 8;
/// The encoded `data` an action may carry to its handler.
const MAX_ACTION_DATA_BYTES: usize = MAX_CONTEXT_BYTES;
const MAX_TOAST_BYTES: usize = 512;

/// What an action asks Dupin to do once it has run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum ActionEffect {
    /// One line Dupin shows briefly in the panel.
    Toast { text: String },
}

/// Reads an action's answer: nothing, or one effect.
pub(crate) fn effect(entry: &str, value: &Value<'_>) -> Result<Option<ActionEffect>, Failure> {
    if value.is_undefined() || value.is_null() {
        return Ok(None);
    }
    let invalid = |field: &str, message: &str| {
        Failure::Report(
            Diagnostic::new("invalid-effect", message)
                .file(entry)
                .field(field)
                .hint("Return nothing, or `{ type: \"toast\", text: \"Saved\" }`."),
        )
    };
    let Some(object) = plain_object(value) else {
        return Err(invalid(
            "effect",
            &format!("is {}, not an effect", describe(value)),
        ));
    };
    for key in object.keys::<String>() {
        let key = key?;
        if key != "type" && key != "text" {
            return Err(invalid(&key, "is not an effect field"));
        }
    }
    let kind: Value = object.get("type")?;
    if kind
        .as_string()
        .map(|kind| kind.to_string())
        .transpose()?
        .as_deref()
        != Some("toast")
    {
        return Err(invalid("type", "must be \"toast\""));
    }
    let text: Value = object.get("text")?;
    let Some(text) = text.as_string().map(|text| text.to_string()).transpose()? else {
        return Err(invalid("text", "must be a string"));
    };
    if text.is_empty() || text.len() > MAX_TOAST_BYTES || printable(&text).is_err() {
        return Err(invalid(
            "text",
            &format!("must be one line of 1–{MAX_TOAST_BYTES} bytes"),
        ));
    }
    Ok(Some(ActionEffect::Toast { text }))
}

const MAX_TITLE_BYTES: usize = 512;
const MAX_SUBTITLE_BYTES: usize = 2048;
const MAX_TEXT_BYTES: usize = 16 * 1024;
const MAX_ID_BYTES: usize = 128;

pub(crate) struct Reader<'a> {
    pub entry: &'a str,
    pub limit: usize,
    bytes: usize,
}

impl<'a> Reader<'a> {
    pub fn new(entry: &'a str, limit: usize) -> Self {
        Self {
            entry,
            limit,
            bytes: 0,
        }
    }

    fn invalid(
        &self,
        code: &'static str,
        field: impl Into<String>,
        message: impl Into<String>,
    ) -> Diagnostic {
        Diagnostic::new(code, message).file(self.entry).field(field)
    }

    pub fn rows<'js>(&mut self, value: Value<'js>) -> Result<Vec<PluginItem>, Failure> {
        let items = if value.is_array() {
            value
        } else if let Some(list) = plain_object(&value) {
            for key in list.keys::<String>() {
                let key = key?;
                if key != "type" && key != "items" {
                    return Err(self
                        .invalid("unknown-field", key, "is not a list field")
                        .hint("A list takes `items`, and optionally `type: \"list\"`.")
                        .into());
                }
            }
            let kind: Value = list.get("type")?;
            if !kind.is_undefined()
                && kind
                    .as_string()
                    .map(|kind| kind.to_string())
                    .transpose()?
                    .as_deref()
                    != Some("list")
            {
                return Err(self
                    .invalid("invalid-field", "type", "must be \"list\"")
                    .hint("Only lists can be returned from `search` for now.")
                    .into());
            }
            let items: Value = list.get("items")?;
            if !items.is_array() {
                return Err(self
                    .invalid("invalid-field", "items", "must be an array of items")
                    .hint("Return `{ items: [{ title: \"…\" }] }`.")
                    .into());
            }
            items
        } else {
            return Err(Diagnostic::new(
                "invalid-result",
                format!(
                    "search returned {}, not an array of items or a list",
                    describe(&value)
                ),
            )
            .file(self.entry)
            .hint("Return an array such as `[{ title: \"Hello\" }]`; return `[]` for no results.")
            .into());
        };
        let items = items.into_array().expect("checked to be an array");
        if items.len() > MAX_ROWS {
            return Err(Diagnostic::new(
                "too-many-rows",
                format!(
                    "search returned {} items; at most {MAX_ROWS} are allowed",
                    items.len()
                ),
            )
            .file(self.entry)
            .hint(format!(
                "Return the best {MAX_ROWS} matches, for example with `.slice(0, {MAX_ROWS})`."
            ))
            .into());
        }
        let mut ids = HashSet::new();
        let mut rows = Vec::with_capacity(items.len().min(self.limit));
        for (index, item) in items.iter::<Value>().enumerate() {
            let row = self.item(index, item?)?;
            if !ids.insert(row.id.clone()) {
                return Err(self
                    .invalid(
                        "duplicate-id",
                        format!("items[{index}].id"),
                        format!("`{}` is used by an earlier item", row.id),
                    )
                    .hint("Give every item a different `id`, or leave `id` out.")
                    .into());
            }
            if rows.len() < self.limit {
                rows.push(row);
            }
        }
        Ok(rows)
    }

    fn item<'js>(&mut self, index: usize, value: Value<'js>) -> Result<PluginItem, Failure> {
        let at = |name: &str| format!("items[{index}].{name}");
        let Some(item) = plain_object(&value) else {
            return Err(self
                .invalid(
                    "invalid-item",
                    format!("items[{index}]"),
                    format!("is {}, not an object", describe(&value)),
                )
                .hint("Each item is an object such as `{ title: \"Hello\" }`.")
                .into());
        };
        for key in item.keys::<String>() {
            let key = key?;
            if !ITEM_FIELDS.contains(&key.as_str()) {
                return Err(self
                    .invalid("unknown-field", at(&key), "is not an item field")
                    .hint("Items take `id`, `title`, `subtitle`, `text`, `url` and `actions`.")
                    .into());
            }
        }
        let title = self
            .string(&item, &at("title"), "title", MAX_TITLE_BYTES)?
            .ok_or_else(|| {
                self.invalid("missing-field", at("title"), "is required")
                    .hint("Give every item a `title` string.")
            })?;
        printable(&title).map_err(|()| {
            self.invalid(
                "invalid-field",
                at("title"),
                "must be one line without control characters",
            )
            .hint("Put further lines in `subtitle`.")
        })?;
        let subtitle = self
            .string(&item, &at("subtitle"), "subtitle", MAX_SUBTITLE_BYTES)?
            .unwrap_or_default();
        let id = match self.string(&item, &at("id"), "id", MAX_ID_BYTES)? {
            Some(id) => {
                printable(&id).map_err(|()| {
                    self.invalid("invalid-field", at("id"), "must not contain control characters")
                        .hint("Use a short, stable name such as `docs`.")
                })?;
                id
            }
            None => format!("#{index}"),
        };
        let mut actions = Vec::new();
        if let Some(url) = self.string(&item, &at("url"), "url", dupin_plugin_protocol::MAX_URL_BYTES)? {
            if !is_web_url(&url) {
                return Err(self
                    .invalid(
                        "invalid-url",
                        at("url"),
                        "must be an http or https URL with a host",
                    )
                    .hint("Use a full URL such as `https://example.com/`; other schemes can't be opened.")
                    .into());
            }
            actions.push(PluginAction {
                id: "open".into(),
                title: "Open URL".into(),
                effect: Effect::OpenUrl { url },
            });
        }
        if let Some(text) = self.string(&item, &at("text"), "text", MAX_TEXT_BYTES)? {
            actions.push(PluginAction {
                id: "copy".into(),
                title: "Copy".into(),
                effect: Effect::Copy { text },
            });
        }
        self.actions(index, &item, &mut actions)?;
        Ok(PluginItem {
            id,
            view: View::Text { title, subtitle },
            actions,
        })
    }

    /// `actions: [{ title, run, data }]`: named handlers in the module's `actions` export that
    /// run when the user chooses them. `data` goes to the handler as JSON.
    fn actions(
        &mut self,
        index: usize,
        item: &Object<'_>,
        actions: &mut Vec<PluginAction>,
    ) -> Result<(), Failure> {
        let value: Value = item.get("actions")?;
        if value.is_undefined() || value.is_null() {
            return Ok(());
        }
        let at = |field: &str| format!("items[{index}].actions{field}");
        let Some(list) = value.as_array() else {
            return Err(self
                .invalid(
                    "invalid-field",
                    at(""),
                    format!("must be an array, not {}", describe(&value)),
                )
                .hint("Give `actions: [{ title: \"Save\", run: \"save\" }]`.")
                .into());
        };
        if actions.len() + list.len() > MAX_ACTIONS {
            return Err(self
                .invalid(
                    "too-many-actions",
                    at(""),
                    format!("an item offers at most {MAX_ACTIONS} actions, `url` and `text` included"),
                )
                .hint("Offer fewer actions.")
                .into());
        }
        for (position, entry) in list.iter::<Value>().enumerate() {
            let entry = entry?;
            let field = |name: &str| at(&format!("[{position}].{name}"));
            let Some(object) = plain_object(&entry) else {
                return Err(self
                    .invalid("invalid-field", at(&format!("[{position}]")), "must be an object")
                    .hint("Give `{ title: \"Save\", run: \"save\" }`.")
                    .into());
            };
            for key in object.keys::<String>() {
                let key = key?;
                if !ACTION_FIELDS.contains(&key.as_str()) {
                    return Err(self
                        .invalid("unknown-field", field(&key), "is not an action field")
                        .hint("Actions take `title`, `run` and `data`.")
                        .into());
                }
            }
            let title = self
                .string(&object, &field("title"), "title", MAX_TITLE_BYTES)?
                .ok_or_else(|| self.invalid("missing-field", field("title"), "is required"))?;
            let run = self
                .string(&object, &field("run"), "run", MAX_HANDLER_BYTES)?
                .ok_or_else(|| {
                    self.invalid("missing-field", field("run"), "is required")
                        .hint("Name a function of the module's `actions` export.")
                })?;
            if printable(&title).is_err() || printable(&run).is_err() {
                return Err(self
                    .invalid(
                        "invalid-field",
                        field("title"),
                        "must be one line without control characters",
                    )
                    .into());
            }
            let data: Value = object.get("data")?;
            let data = if data.is_undefined() {
                None
            } else {
                let encoded = data
                    .ctx()
                    .json_stringify(data.clone())?
                    .map(|text| text.to_string())
                    .transpose()?;
                let parsed = encoded
                    .as_deref()
                    .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
                    .filter(|value| fits(value, MAX_CONTEXT_DEPTH));
                match parsed {
                    Some(value)
                        if encoded
                            .as_ref()
                            .is_some_and(|text| text.len() <= MAX_ACTION_DATA_BYTES) =>
                    {
                        self.bytes += encoded.map_or(0, |text| text.len());
                        Some(value)
                    }
                    _ => {
                        return Err(self
                            .invalid(
                                "invalid-field",
                                field("data"),
                                format!(
                                    "must be JSON of at most {MAX_ACTION_DATA_BYTES} bytes, nested at most {MAX_CONTEXT_DEPTH} deep"
                                ),
                            )
                            .hint("Pass an ID or a small object, and look the rest up in the action.")
                            .into());
                    }
                }
            };
            let id = format!("run-{position}");
            actions.push(PluginAction {
                id,
                title,
                effect: Effect::Run { action: run, data },
            });
        }
        Ok(())
    }

    /// An optional string field: absent, `undefined` and `null` are all none. Counts the bytes
    /// toward the whole answer's limit.
    fn string(
        &mut self,
        item: &Object<'_>,
        field: &str,
        key: &str,
        cap: usize,
    ) -> Result<Option<String>, Failure> {
        let value: Value = item.get(key)?;
        if value.is_undefined() || value.is_null() {
            return Ok(None);
        }
        let Some(string) = value.as_string() else {
            return Err(self
                .invalid(
                    "invalid-field",
                    field,
                    format!("must be a string, not {}", describe(&value)),
                )
                .hint("Convert values with `String(value)`.")
                .into());
        };
        let string = string.to_string()?;
        if string.is_empty() && key != "subtitle" {
            return Err(self
                .invalid("invalid-field", field, "is empty")
                .hint(format!("Leave `{key}` out instead of giving an empty string."))
                .into());
        }
        if string.len() > cap {
            return Err(self
                .invalid(
                    "field-too-long",
                    field,
                    format!("is {} bytes; at most {cap} are allowed", string.len()),
                )
                .hint("Shorten the value.")
                .into());
        }
        self.bytes += string.len();
        if self.bytes > MAX_RESPONSE_BYTES {
            return Err(Diagnostic::new(
                "output-too-large",
                format!(
                    "search returned more than {} KiB of text",
                    MAX_RESPONSE_BYTES / 1024
                ),
            )
            .file(self.entry)
            .field(field)
            .hint("Return fewer or shorter items.")
            .into());
        }
        Ok(Some(string))
    }
}

fn fits(value: &serde_json::Value, remaining: usize) -> bool {
    match value {
        serde_json::Value::Array(values) => remaining > 0 && values.iter().all(|v| fits(v, remaining - 1)),
        serde_json::Value::Object(values) => remaining > 0 && values.values().all(|v| fits(v, remaining - 1)),
        _ => true,
    }
}

fn printable(value: &str) -> Result<(), ()> {
    if value.chars().any(char::is_control) {
        Err(())
    } else {
        Ok(())
    }
}

fn plain_object<'js>(value: &Value<'js>) -> Option<Object<'js>> {
    if value.is_array() || value.is_function() || value.is_promise() {
        return None;
    }
    value.as_object().cloned()
}

pub(crate) fn describe(value: &Value<'_>) -> &'static str {
    if value.is_undefined() {
        "undefined"
    } else if value.is_null() {
        "null"
    } else if value.is_bool() {
        "a boolean"
    } else if value.is_number() {
        "a number"
    } else if value.is_string() {
        "a string"
    } else if value.is_array() {
        "an array"
    } else if value.is_function() {
        "a function"
    } else if value.is_promise() {
        "a promise"
    } else if value.is_object() {
        "an object"
    } else {
        "an unsupported value"
    }
}
