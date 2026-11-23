//! Reads what `search` returned into protocol rows, checking every value against the same
//! limits a Wasm guest's response meets.

use std::collections::HashSet;

use dupin_plugin_protocol::{Effect, MAX_RESPONSE_BYTES, PluginAction, PluginItem, View, is_web_url};
use rquickjs::{Object, Value};

use crate::{Diagnostic, Failure, MAX_ROWS};

const ITEM_FIELDS: [&str; 5] = ["id", "title", "subtitle", "text", "url"];
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
                    .hint("Items take `id`, `title`, `subtitle`, `text` and `url`.")
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
        Ok(PluginItem {
            id,
            view: View::Text { title, subtitle },
            actions,
        })
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
