//! Reads what `search` or `items` returned into rows, checking every value against the limits
//! in [`crate::row`] and naming the field at fault.

use std::collections::HashSet;

use dupin_plugin_protocol::{MAX_RESPONSE_BYTES, MAX_URL_BYTES};
use rquickjs::{Array, Object, Value};
use serde::{Deserialize, Serialize};

use crate::row::{
    self, Accessory, CataloguePage, CopiedFileImage, MAX_ACCESSORIES, MAX_ACCESSORY_BYTES,
    MAX_ACTION_TEXT_BYTES, MAX_ACTION_TITLE_BYTES, MAX_ACTIONS, MAX_CURSOR_BYTES, MAX_FILE_BYTES,
    MAX_ID_BYTES, MAX_IMAGE_HANDLE_BYTES, MAX_KEYWORD_BYTES, MAX_KEYWORDS, MAX_LABEL_BYTES,
    MAX_MARKDOWN_BYTES, MAX_MATCH_TEXT_BYTES, MAX_METADATA, MAX_METADATA_TEXT_BYTES, MAX_PAGE_BYTES,
    MAX_PAGE_ROWS, MAX_SECTION_BYTES, MAX_SUBTITLE_BYTES, MAX_TEXT_BYTES, MAX_TIME_MS, MAX_TITLE_BYTES,
    MAX_TTL_SECONDS, Metadata, PageChange, Row, RowAction, RowDetail, RowEffect, RowImage, Source,
};
use crate::view::{
    self, Field, FieldKind, Form, MAX_EMPTY_TEXT_BYTES, MAX_FIELD_ID_BYTES, MAX_FIELD_TITLE_BYTES,
    MAX_FIELDS, MAX_OPTION_BYTES, MAX_OPTIONS, MAX_TEXT_FIELD_BYTES, MAX_TEXTAREA_BYTES, SelectOption,
    ViewAnswer,
};
use crate::web_search::{self, WebSearch};
use crate::{Diagnostic, Failure, MAX_ROWS};

/// `key` of the object at `field`, or `key` alone at the top of an answer.
fn join(field: &str, key: &str) -> String {
    if field.is_empty() {
        key.to_owned()
    } else {
        format!("{field}.{key}")
    }
}

const MAX_TOAST_BYTES: usize = 512;

/// What an action asks Dupin to do once it has run. The effects that move between pages or
/// change one may carry a `toast` too, which Dupin shows once it has done so.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum ActionEffect {
    /// One line Dupin shows briefly in the panel.
    Toast { text: String },
    /// Hide the panel, as copying does.
    Hide,
    /// Hide the panel and paste what the action put on the clipboard into the app in front.
    Paste,
    /// Open the package's `views[view]` on a new page above the one the action ran from.
    Push {
        view: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        toast: Option<String>,
    },
    /// Close the page the action ran from.
    Pop {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        toast: Option<String>,
    },
    /// Ask the page's view, or the search, again.
    Refresh {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        toast: Option<String>,
    },
    /// Give the page the action ran from this state, and ask its view again.
    ReplaceState {
        state: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        toast: Option<String>,
    },
    /// What is wrong with a form's fields, by field ID, which Dupin shows under them.
    FieldErrors {
        errors: std::collections::BTreeMap<String, String>,
    },
}

impl ActionEffect {
    /// The line Dupin shows once the effect is carried out.
    pub fn toast(&self) -> Option<&str> {
        match self {
            ActionEffect::Toast { text } => Some(text),
            ActionEffect::Push { toast, .. }
            | ActionEffect::Pop { toast }
            | ActionEffect::Refresh { toast }
            | ActionEffect::ReplaceState { toast, .. } => toast.as_deref(),
            ActionEffect::Hide | ActionEffect::Paste | ActionEffect::FieldErrors { .. } => None,
        }
    }

    /// Checks the limits the reader enforced, for an effect from another process.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(toast) = self.toast()
            && (toast.is_empty() || toast.len() > MAX_TOAST_BYTES || printable(toast).is_err())
        {
            return Err(format!("a toast must be one line of 1–{MAX_TOAST_BYTES} bytes"));
        }
        match self {
            ActionEffect::Push {
                view, title, state, ..
            } => {
                row::check_name(view, "view")?;
                if let Some(title) = title
                    && (title.is_empty() || title.len() > MAX_ACTION_TITLE_BYTES || printable(title).is_err())
                {
                    return Err("a pushed page's `title` must be one line".into());
                }
                if let Some(state) = state {
                    row::check_state(state)?;
                }
            }
            ActionEffect::ReplaceState { state, .. } => row::check_state(state)?,
            ActionEffect::FieldErrors { errors } => {
                if errors.is_empty() || errors.len() > crate::view::MAX_FIELDS {
                    return Err(format!(
                        "`fieldErrors` names 1 to {} fields",
                        crate::view::MAX_FIELDS
                    ));
                }
                for (id, error) in errors {
                    crate::view::check_field_id(id)
                        .map_err(|message| format!("an error's field {message}"))?;
                    if error.is_empty()
                        || error.len() > crate::view::MAX_FIELD_ERROR_BYTES
                        || printable(error).is_err()
                    {
                        return Err(format!(
                            "a field's error must be one line of 1–{} bytes",
                            crate::view::MAX_FIELD_ERROR_BYTES
                        ));
                    }
                }
            }
            ActionEffect::Toast { .. }
            | ActionEffect::Hide
            | ActionEffect::Paste
            | ActionEffect::Pop { .. }
            | ActionEffect::Refresh { .. } => {}
        }
        Ok(())
    }
}

const EFFECT_HINT: &str = "Return nothing, or one of `{ type: \"toast\", text }`, `{ type: \"hide\" }`, `{ type: \"paste\" }`, `{ type: \"push\", view, title?, state? }`, `{ type: \"pop\" }`, `{ type: \"refresh\" }`, `{ type: \"replaceState\", state }` and `{ type: \"fieldErrors\", errors }`; the ones that move or change a page take a `toast` too.";

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
                .hint(EFFECT_HINT),
        )
    };
    let Some(object) = plain_object(value) else {
        return Err(invalid(
            "effect",
            &format!("is {}, not an effect", describe(value)),
        ));
    };
    let kind: Value = object.get("type")?;
    let kind = kind.as_string().map(|kind| kind.to_string()).transpose()?;
    let allowed: &[&str] = match kind.as_deref() {
        Some("toast") => &["type", "text"],
        Some("hide" | "paste") => &["type"],
        Some("push") => &["type", "view", "title", "state", "toast"],
        Some("pop" | "refresh") => &["type", "toast"],
        Some("replaceState") => &["type", "state", "toast"],
        Some("fieldErrors") => &["type", "errors"],
        _ => {
            return Err(invalid(
                "type",
                "must be \"toast\", \"hide\", \"paste\", \"push\", \"pop\", \"refresh\", \"replaceState\" or \"fieldErrors\"",
            ));
        }
    };
    for key in object.keys::<String>() {
        let key = key?;
        if !allowed.contains(&key.as_str()) {
            return Err(invalid(&key, "is not a field of this effect"));
        }
    }
    let line = |key: &str, cap: usize| -> Result<Option<String>, Failure> {
        let value: Value = object.get(key)?;
        if value.is_undefined() {
            return Ok(None);
        }
        let Some(text) = value.as_string().map(|text| text.to_string()).transpose()? else {
            return Err(invalid(key, "must be a string"));
        };
        if text.is_empty() || text.len() > cap || printable(&text).is_err() {
            return Err(invalid(key, &format!("must be one line of 1–{cap} bytes")));
        }
        Ok(Some(text))
    };
    let state = |required: bool| -> Result<Option<serde_json::Value>, Failure> {
        let value: Value = object.get("state")?;
        if value.is_undefined() {
            return if required {
                Err(invalid("state", "is required"))
            } else {
                Ok(None)
            };
        }
        let state = json(&value).ok_or_else(|| {
            invalid(
                "state",
                "must be JSON: strings, numbers, booleans, null, arrays or objects",
            )
        })?;
        row::check_state(&state).map_err(|message| invalid("state", &message.replace("`state` ", "")))?;
        Ok(Some(state))
    };
    let effect = match kind.as_deref().expect("matched above") {
        "toast" => ActionEffect::Toast {
            text: line("text", MAX_TOAST_BYTES)?.ok_or_else(|| invalid("text", "must be a string"))?,
        },
        "hide" => ActionEffect::Hide,
        "paste" => ActionEffect::Paste,
        "push" => {
            let view = line("view", row::MAX_HANDLER_BYTES)?.ok_or_else(|| invalid("view", "is required"))?;
            row::check_name(&view, "view")
                .map_err(|message| invalid("view", &message.replace("`view` ", "")))?;
            ActionEffect::Push {
                view,
                title: line("title", MAX_ACTION_TITLE_BYTES)?,
                state: state(false)?,
                toast: line("toast", MAX_TOAST_BYTES)?,
            }
        }
        "pop" => ActionEffect::Pop {
            toast: line("toast", MAX_TOAST_BYTES)?,
        },
        "refresh" => ActionEffect::Refresh {
            toast: line("toast", MAX_TOAST_BYTES)?,
        },
        "replaceState" => ActionEffect::ReplaceState {
            state: state(true)?.expect("required"),
            toast: line("toast", MAX_TOAST_BYTES)?,
        },
        _ => {
            let errors: Value = object.get("errors")?;
            let Some(errors) = plain_object(&errors) else {
                return Err(invalid("errors", "must be an object of messages by field ID"));
            };
            let mut read = std::collections::BTreeMap::new();
            for key in errors.keys::<String>() {
                let key = key?;
                if read.len() == crate::view::MAX_FIELDS {
                    return Err(invalid(
                        "errors",
                        &format!("names more than {} fields", crate::view::MAX_FIELDS),
                    ));
                }
                let message: Value = errors.get(key.as_str())?;
                let message = message.as_string().map(|text| text.to_string()).transpose()?;
                read.insert(key, message.unwrap_or_default());
            }
            let effect = ActionEffect::FieldErrors { errors: read };
            effect.validate().map_err(|message| invalid("errors", &message))?;
            effect
        }
    };
    Ok(Some(effect))
}

/// `value` as JSON, as `JSON.stringify` writes it, or none when it has no JSON form.
fn json(value: &Value<'_>) -> Option<serde_json::Value> {
    let ctx = value.ctx().clone();
    let text = ctx.json_stringify(value.clone()).ok()??.to_string().ok()?;
    serde_json::from_str(&text).ok()
}

const ITEM_FIELDS: [&str; 12] = [
    "id",
    "title",
    "subtitle",
    "text",
    "url",
    "icon",
    "image",
    "section",
    "detail",
    "actions",
    "accessories",
    "answer",
];
const ITEM_FIELDS_HINT: &str = "Items take `id`, `title`, `subtitle`, `text`, `url`, `icon`, `image`, `section`, `detail`, `actions`, `accessories` and `answer`.";
const CATALOGUE_ITEM_FIELDS_HINT: &str = "Items take `id`, `title`, `subtitle`, `aliases`, `keywords`, `matchText`, `time`, `file`, `text`, `url`, `icon`, `image`, `section`, `detail`, `actions`, `accessories` and `webSearch`.";
const ACCESSORY_HINT: &str = "Give up to three `{ text: \"2 KB\" }` or `{ tag: \"New\" }`.";
const IMAGE_HINT: &str = "Give a stored blob's `handle`, or a copied file's `{ path, handle }` as the clipboard watch delivered it.";
const WEB_SEARCH_HINT: &str = "Give `webSearch` an `alias` such as `\"g\"`, a `url` such as `\"https://www.google.com/search?q={query}\"`, a `title` such as `\"Search Google for {query}\"` and optionally a `fallback` position.";
/// Fields only an `items()` catalogue's items take.
const CATALOGUE_ONLY_FIELDS: [&str; 6] = ["aliases", "keywords", "matchText", "time", "file", "webSearch"];

pub(crate) struct Reader<'a> {
    pub entry: &'a str,
    pub limit: usize,
    source: Source,
    /// What answered, as messages name it.
    name: &'static str,
    bytes: usize,
}

impl<'a> Reader<'a> {
    pub fn search(entry: &'a str, limit: usize) -> Self {
        Self {
            entry,
            limit,
            source: Source::Search,
            name: "search",
            bytes: 0,
        }
    }

    pub fn items(entry: &'a str) -> Self {
        Self {
            entry,
            limit: MAX_PAGE_ROWS,
            source: Source::Items,
            name: "items",
            bytes: 0,
        }
    }

    /// A view's list is read as `search`'s is.
    pub fn view(entry: &'a str) -> Self {
        Self {
            name: "the view",
            ..Self::search(entry, MAX_ROWS)
        }
    }

    fn max_bytes(&self) -> usize {
        match self.source {
            Source::Search => MAX_RESPONSE_BYTES,
            Source::Items => MAX_PAGE_BYTES,
        }
    }

    fn max_rows(&self) -> usize {
        match self.source {
            Source::Search => MAX_ROWS,
            Source::Items => MAX_PAGE_ROWS,
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

    /// What `search` returned: an array of items, or a list of items and sections.
    pub fn rows(&mut self, value: Value<'_>) -> Result<Vec<Row>, Failure> {
        if value.is_array() {
            return self.list_items(value, "items", None);
        }
        let Some(list) = plain_object(&value) else {
            return Err(self.not_a_list(&value));
        };
        self.only_keys(
            &list,
            &["type", "items", "sections"],
            "A list takes `items` or `sections`, and optionally `type: \"list\"`.",
        )?;
        self.kind(&list, &["list"])?;
        self.list(&list)
    }

    /// What `items` returned: the whole catalogue, or changes to the one Dupin holds.
    pub fn page(&mut self, value: Value<'_>) -> Result<CataloguePage, Failure> {
        if value.is_array() {
            let rows = self.list_items(value, "items", None)?;
            return Ok(CataloguePage {
                change: PageChange::Reset { rows },
                ttl: None,
                cursor: None,
                more: false,
            });
        }
        let Some(answer) = plain_object(&value) else {
            return Err(self.not_a_list(&value));
        };
        let update = self.kind(&answer, &["list", "update"])? == Some("update");
        let more = self.boolean(&answer, "more")?.unwrap_or(false);
        // The cursor isn't the items' text, which alone the answer's limit weighs.
        let cursor = self.uncounted(&answer, "cursor", "cursor", MAX_CURSOR_BYTES)?;
        let ttl = self.ttl(&answer)?;
        let change = if update {
            self.only_keys(
                &answer,
                &["type", "prepend", "append", "replace", "remove", "ttl", "cursor", "more"],
                "An update takes `prepend`, `append`, `replace` and `remove`, and optionally `ttl`, `cursor` and `more`.",
            )?;
            let prepend = self.optional_items(&answer, "prepend")?;
            let append = self.optional_items(&answer, "append")?;
            let replace = self.optional_items(&answer, "replace")?;
            let rows = prepend.len() + append.len() + replace.len();
            if rows > MAX_PAGE_ROWS {
                return Err(self.too_many_rows(rows));
            }
            let mut ids = HashSet::new();
            for (field, row) in prepend
                .iter()
                .enumerate()
                .map(|(i, row)| (format!("prepend[{i}].id"), row))
                .chain(
                    append
                        .iter()
                        .enumerate()
                        .map(|(i, row)| (format!("append[{i}].id"), row)),
                )
                .chain(
                    replace
                        .iter()
                        .enumerate()
                        .map(|(i, row)| (format!("replace[{i}].id"), row)),
                )
            {
                if !ids.insert(row.id.as_str()) {
                    return Err(self.duplicate(field, &row.id));
                }
            }
            let remove = self.removed(&answer)?;
            PageChange::Update {
                prepend,
                append,
                replace,
                remove,
            }
        } else {
            self.only_keys(
                &answer,
                &["type", "items", "sections", "ttl", "cursor", "more"],
                "A list takes `items` or `sections`, and optionally `type: \"list\"`, `ttl`, `cursor` and `more`.",
            )?;
            PageChange::Reset {
                rows: self.list(&answer)?,
            }
        };
        Ok(CataloguePage {
            change,
            ttl,
            cursor,
            more,
        })
    }

    fn not_a_list(&self, value: &Value<'_>) -> Failure {
        let name = self.name;
        Diagnostic::new(
            "invalid-result",
            format!(
                "{name} returned {}, not an array of items or a list",
                describe(value)
            ),
        )
        .file(self.entry)
        .hint("Return an array such as `[{ title: \"Hello\" }]`; return `[]` for no results.")
        .into()
    }

    fn only_keys(&self, object: &Object<'_>, allowed: &[&str], hint: &str) -> Result<(), Failure> {
        for key in object.keys::<String>() {
            let key = key?;
            if !allowed.contains(&key.as_str()) {
                return Err(self
                    .invalid("unknown-field", key, "is not a field of this answer")
                    .hint(hint)
                    .into());
            }
        }
        Ok(())
    }

    /// The answer's `type`, which must be one of `allowed` when present.
    fn kind(&self, object: &Object<'_>, allowed: &[&'static str]) -> Result<Option<&'static str>, Failure> {
        let kind: Value = object.get("type")?;
        if kind.is_undefined() {
            return Ok(None);
        }
        let text = kind.as_string().map(|kind| kind.to_string()).transpose()?;
        match allowed.iter().find(|allowed| Some(**allowed) == text.as_deref()) {
            Some(found) => Ok(Some(*found)),
            None => {
                let quoted: Vec<String> = allowed.iter().map(|kind| format!("\"{kind}\"")).collect();
                Err(self
                    .invalid(
                        "invalid-field",
                        "type",
                        format!("must be {}", quoted.join(" or ")),
                    )
                    .hint(match self.source {
                        Source::Search => "Only lists can be returned from `search` for now.",
                        Source::Items => {
                            "Return a list of the whole catalogue, or `type: \"update\"` with changes."
                        }
                    })
                    .into())
            }
        }
    }

    /// A list's rows: its `items`, or the rows of its `sections` in order, each under its title.
    fn list(&mut self, list: &Object<'_>) -> Result<Vec<Row>, Failure> {
        let items: Value = list.get("items")?;
        let sections: Value = list.get("sections")?;
        if !sections.is_undefined() {
            if !items.is_undefined() {
                return Err(self
                    .invalid("invalid-field", "items", "can't be given with `sections`")
                    .hint("Put every item in a section, or leave `sections` out.")
                    .into());
            }
            let Some(sections) = sections.as_array() else {
                return Err(self
                    .invalid("invalid-field", "sections", "must be an array of sections")
                    .hint("Return `{ sections: [{ title: \"Recent\", items: [] }] }`.")
                    .into());
            };
            let total = self.section_rows(sections)?;
            if total > self.max_rows() {
                return Err(self.too_many_rows(total));
            }
            let mut rows = Vec::new();
            let mut ids = HashSet::new();
            for (s, section) in sections.iter::<Value>().enumerate() {
                let field = format!("sections[{s}]");
                let section = section?;
                let Some(section) = plain_object(&section) else {
                    return Err(self
                        .invalid(
                            "invalid-field",
                            &field,
                            "must be an object with `title` and `items`",
                        )
                        .hint("Each section is `{ title: \"Recent\", items: [] }`.")
                        .into());
                };
                self.only_keys(
                    &section,
                    &["title", "items"],
                    "A section takes `title` and `items`.",
                )?;
                let title = self
                    .line(&section, &format!("{field}.title"), "title", MAX_SECTION_BYTES)?
                    .ok_or_else(|| {
                        self.invalid("missing-field", format!("{field}.title"), "is required")
                            .hint("Give every section a `title`.")
                    })?;
                let items: Value = section.get("items")?;
                let start = rows.len();
                let read =
                    self.list_items_with(items, &format!("{field}.items"), Some(&title), start, &mut ids)?;
                rows.extend(read);
            }
            rows.truncate(self.limit);
            return Ok(rows);
        }
        if items.is_undefined() && self.source == Source::Items {
            return Ok(Vec::new());
        }
        if !items.is_array() {
            return Err(self
                .invalid("invalid-field", "items", "must be an array of items")
                .hint("Return `{ items: [{ title: \"…\" }] }`.")
                .into());
        }
        self.list_items(items, "items", None)
    }

    fn section_rows(&self, sections: &Array<'_>) -> Result<usize, Failure> {
        let mut total = 0;
        for section in sections.iter::<Value>() {
            let section = section?;
            if let Some(section) = plain_object(&section) {
                let items: Value = section.get("items")?;
                total += items.as_array().map_or(0, Array::len);
            }
        }
        Ok(total)
    }

    fn list_items(
        &mut self,
        items: Value<'_>,
        path: &str,
        section: Option<&str>,
    ) -> Result<Vec<Row>, Failure> {
        let mut ids = HashSet::new();
        let mut rows = self.list_items_with(items, path, section, 0, &mut ids)?;
        rows.truncate(self.limit);
        Ok(rows)
    }

    /// Reads `items` at `path`, numbering items without an ID from `start`. The array's length
    /// is checked before anything is allocated for it, since a sparse array claims any length
    /// without costing the package's heap.
    fn list_items_with(
        &mut self,
        items: Value<'_>,
        path: &str,
        section: Option<&str>,
        start: usize,
        ids: &mut HashSet<String>,
    ) -> Result<Vec<Row>, Failure> {
        let Some(items) = items.into_array() else {
            return Err(self
                .invalid("invalid-field", path, "must be an array of items")
                .hint("Return `{ items: [{ title: \"…\" }] }`.")
                .into());
        };
        if items.len() > self.max_rows() {
            return Err(self.too_many_rows(items.len()));
        }
        let mut rows = Vec::with_capacity(items.len());
        for (index, item) in items.iter::<Value>().enumerate() {
            let field = format!("{path}[{index}]");
            let mut row = self.item(&field, start + index, item?)?;
            if row.section.is_none() {
                row.section = section.map(str::to_owned);
            }
            if !ids.insert(row.id.clone()) {
                return Err(self.duplicate(format!("{field}.id"), &row.id));
            }
            rows.push(row);
        }
        Ok(rows)
    }

    fn optional_items(&mut self, object: &Object<'_>, key: &str) -> Result<Vec<Row>, Failure> {
        let items: Value = object.get(key)?;
        if items.is_undefined() {
            return Ok(Vec::new());
        }
        let mut ids = HashSet::new();
        self.list_items_with(items, key, None, 0, &mut ids)
    }

    fn removed(&mut self, object: &Object<'_>) -> Result<Vec<String>, Failure> {
        let remove: Value = object.get("remove")?;
        if remove.is_undefined() {
            return Ok(Vec::new());
        }
        let Some(remove) = remove.into_array() else {
            return Err(self
                .invalid("invalid-field", "remove", "must be an array of item IDs")
                .hint("Return `{ type: \"update\", remove: [\"id\"] }`.")
                .into());
        };
        if remove.len() > MAX_PAGE_ROWS {
            return Err(self.too_many_rows(remove.len()));
        }
        let mut ids = Vec::with_capacity(remove.len());
        for (index, id) in remove.iter::<Value>().enumerate() {
            let field = format!("remove[{index}]");
            let id = id?;
            let Some(text) = id.as_string() else {
                return Err(self
                    .invalid(
                        "invalid-field",
                        field,
                        format!("must be an item ID, not {}", describe(&id)),
                    )
                    .hint("List the `id` of each item to remove.")
                    .into());
            };
            let text = text.to_string()?;
            self.count(&field, &text, MAX_ID_BYTES)?;
            ids.push(text);
        }
        Ok(ids)
    }

    fn ttl(&self, object: &Object<'_>) -> Result<Option<u32>, Failure> {
        let value: Value = object.get("ttl")?;
        if value.is_undefined() {
            return Ok(None);
        }
        let seconds = value.as_number().filter(|seconds| {
            seconds.fract() == 0.0 && *seconds >= 1.0 && *seconds <= f64::from(MAX_TTL_SECONDS)
        });
        match seconds {
            Some(seconds) => Ok(Some(seconds as u32)),
            None => Err(self
                .invalid(
                    "invalid-field",
                    "ttl",
                    format!("must be a whole number of seconds from 1 to {MAX_TTL_SECONDS}"),
                )
                .hint("Leave `ttl` out to keep the catalogue until an action invalidates it.")
                .into()),
        }
    }

    fn boolean(&self, object: &Object<'_>, key: &str) -> Result<Option<bool>, Failure> {
        let value: Value = object.get(key)?;
        if value.is_undefined() {
            return Ok(None);
        }
        value.as_bool().map(Some).ok_or_else(|| {
            self.invalid(
                "invalid-field",
                key,
                format!("must be a boolean, not {}", describe(&value)),
            )
            .hint(format!("Set `{key}: true` or leave it out."))
            .into()
        })
    }

    fn too_many_rows(&self, rows: usize) -> Failure {
        let max = self.max_rows();
        let name = self.name;
        let hint = match self.source {
            Source::Search => format!("Return the best {max} matches, for example with `.slice(0, {max})`."),
            Source::Items => format!(
                "Answer with the first {max} and `more: true`; Dupin asks again with your `cursor` for the rest."
            ),
        };
        Diagnostic::new(
            "too-many-rows",
            format!("{name} returned {rows} items; at most {max} are allowed"),
        )
        .file(self.entry)
        .hint(hint)
        .into()
    }

    fn duplicate(&self, field: String, id: &str) -> Failure {
        self.invalid(
            "duplicate-id",
            field,
            format!("`{id}` is used by an earlier item"),
        )
        .hint(match self.source {
            Source::Search => "Give every item a different `id`, or leave `id` out.",
            Source::Items => "Give every item a different `id`.",
        })
        .into()
    }

    fn item(&mut self, at: &str, index: usize, value: Value<'_>) -> Result<Row, Failure> {
        let field = |name: &str| format!("{at}.{name}");
        let Some(item) = plain_object(&value) else {
            return Err(self
                .invalid(
                    "invalid-item",
                    at,
                    format!("is {}, not an object", describe(&value)),
                )
                .hint("Each item is an object such as `{ title: \"Hello\" }`.")
                .into());
        };
        for key in item.keys::<String>() {
            let key = key?;
            let catalogue_only = CATALOGUE_ONLY_FIELDS.contains(&key.as_str());
            if key == "answer" && self.source == Source::Items {
                return Err(self
                    .invalid("unknown-field", field(&key), "is not a field of a catalogue's items")
                    .hint("An item shows its title and `answer` as two values only from `search` or a view, which compute it for the query.")
                    .into());
            }
            if !ITEM_FIELDS.contains(&key.as_str()) && !(catalogue_only && self.source == Source::Items) {
                let hint = match self.source {
                    Source::Search if key == "webSearch" => {
                        "Dupin fills a `webSearch` as the user types; export `items` to list one."
                    }
                    Source::Search if catalogue_only => {
                        "`aliases`, `keywords`, `matchText`, `time` and `file` describe items Dupin holds; export `items` to use them."
                    }
                    Source::Search => ITEM_FIELDS_HINT,
                    Source::Items => CATALOGUE_ITEM_FIELDS_HINT,
                };
                return Err(self
                    .invalid("unknown-field", field(&key), "is not an item field")
                    .hint(hint)
                    .into());
            }
        }
        let title = self
            .string(&item, &field("title"), "title", MAX_TITLE_BYTES)?
            .ok_or_else(|| {
                self.invalid("missing-field", field("title"), "is required")
                    .hint("Give every item a `title` string.")
            })?;
        printable(&title).map_err(|()| {
            self.invalid(
                "invalid-field",
                field("title"),
                "must be one line without control characters",
            )
            .hint("Put further lines in `subtitle`.")
        })?;
        let subtitle = self
            .string(&item, &field("subtitle"), "subtitle", MAX_SUBTITLE_BYTES)?
            .unwrap_or_default();
        let id = match self.string(&item, &field("id"), "id", MAX_ID_BYTES)? {
            Some(id) => {
                printable(&id).map_err(|()| {
                    self.invalid(
                        "invalid-field",
                        field("id"),
                        "must not contain control characters",
                    )
                    .hint("Use a short, stable name such as `docs`.")
                })?;
                id
            }
            None if self.source == Source::Items => {
                return Err(self
                    .invalid("missing-field", field("id"), "is required in a catalogue")
                    .hint("Dupin updates a catalogue and remembers the items you pick by `id`; give each item one that stays with it, such as its URL or a database key.")
                    .into());
            }
            None => format!("#{index}"),
        };
        let aliases = self.words(&item, &field("aliases"), "aliases")?;
        let keywords = self.words(&item, &field("keywords"), "keywords")?;
        let match_text = self.string(&item, &field("matchText"), "matchText", MAX_MATCH_TEXT_BYTES)?;
        let time = self.time(&item, &field("time"))?;
        let file = self.string(&item, &field("file"), "file", MAX_FILE_BYTES)?;
        if let Some(file) = &file {
            row::check_file(file).map_err(|message| {
                self.invalid(
                    "invalid-field",
                    field("file"),
                    message.replace("`file` must", "must"),
                )
                .hint("Give the full path, such as `/Users/me/Report.pdf`.")
            })?;
        }
        let icon = self.string(&item, &field("icon"), "icon", row::MAX_ICON_BYTES)?;
        if let Some(icon) = &icon {
            row::check_icon(icon).map_err(|message| {
                self.invalid("invalid-field", field("icon"), message.replace("`icon` must", "must"))
                    .hint("Ship the image in the package and name it relative to `package.json`, such as `assets/icon.png`.")
            })?;
        }
        let image = self.image(&item, &field("image"))?;
        let section = self.line(&item, &field("section"), "section", MAX_SECTION_BYTES)?;
        let detail = self.detail(&item, &field("detail"))?;
        let mut actions = Vec::new();
        if let Some(url) = self.string(&item, &field("url"), "url", MAX_URL_BYTES)? {
            self.openable_url(&field("url"), &url)?;
            actions.push(RowAction::new("Open URL", RowEffect::OpenUrl { url }));
        }
        if let Some(text) = self.string(&item, &field("text"), "text", MAX_TEXT_BYTES)? {
            actions.push(RowAction::new("Copy", RowEffect::Copy { text }));
        }
        self.actions(&item, &field("actions"), &mut actions)?;
        let web_search = self.web_search(&item, &field("webSearch"))?;
        if web_search.is_some() && !actions.is_empty() {
            return Err(self
                .invalid(
                    "invalid-field",
                    field("webSearch"),
                    "is on an item that also has `url`, `text` or `actions`",
                )
                .hint(
                    "Dupin opens a web search's own URL; leave `url`, `text` and `actions` out of its item.",
                )
                .into());
        }
        let accessories = self.accessories(&item, &field("accessories"))?;
        let answer = self.answer(&item, &field("answer"))?;
        Ok(Row {
            accessories,
            answer,
            id,
            title,
            subtitle,
            aliases,
            keywords,
            match_text,
            time,
            file,
            icon,
            image,
            section,
            detail,
            actions,
            web_search,
        })
    }

    /// Up to three `{ text }` or `{ tag }`, each one line.
    fn accessories(&mut self, item: &Object<'_>, field: &str) -> Result<Vec<Accessory>, Failure> {
        let value: Value = item.get("accessories")?;
        if value.is_undefined() || value.is_null() {
            return Ok(Vec::new());
        }
        let Some(list) = value.into_array() else {
            return Err(self
                .invalid("invalid-field", field, "must be an array of accessories")
                .hint(ACCESSORY_HINT)
                .into());
        };
        if list.len() > MAX_ACCESSORIES {
            return Err(self
                .invalid(
                    "invalid-field",
                    field,
                    format!("has more than {MAX_ACCESSORIES} accessories"),
                )
                .hint(ACCESSORY_HINT)
                .into());
        }
        let mut accessories = Vec::with_capacity(list.len());
        for (index, accessory) in list.iter::<Value>().enumerate() {
            let at = format!("{field}[{index}]");
            let accessory = accessory?;
            let shape = |reader: &Self| {
                Failure::from(
                    reader
                        .invalid("invalid-field", &at, "must be `{ text }` or `{ tag }`")
                        .hint(ACCESSORY_HINT),
                )
            };
            let Some(object) = plain_object(&accessory) else {
                return Err(shape(self));
            };
            let keys: Vec<String> = object.keys::<String>().collect::<rquickjs::Result<_>>()?;
            let [key] = keys.as_slice() else {
                return Err(shape(self));
            };
            if key != "text" && key != "tag" {
                return Err(shape(self));
            }
            let Some(text) = self.line(&object, &format!("{at}.{key}"), key, MAX_ACCESSORY_BYTES)? else {
                return Err(shape(self));
            };
            accessories.push(if key == "tag" {
                Accessory::Tag(text)
            } else {
                Accessory::Text(text)
            });
        }
        Ok(accessories)
    }

    fn web_search(&mut self, item: &Object<'_>, field: &str) -> Result<Option<WebSearch>, Failure> {
        let value: Value = item.get("webSearch")?;
        if value.is_undefined() || value.is_null() {
            return Ok(None);
        }
        let Some(object) = plain_object(&value) else {
            return Err(self
                .invalid(
                    "invalid-field",
                    field,
                    format!("must be an object, not {}", describe(&value)),
                )
                .hint(WEB_SEARCH_HINT)
                .into());
        };
        self.only_keys(&object, &["alias", "url", "title", "fallback"], WEB_SEARCH_HINT)
            .map_err(|failure| self.nested(failure, field))?;
        let required = |reader: &mut Self, key: &str, cap: usize| {
            let at = format!("{field}.{key}");
            reader.string(&object, &at, key, cap)?.ok_or_else(|| {
                Failure::from(
                    reader
                        .invalid("missing-field", at, "is required")
                        .hint(WEB_SEARCH_HINT),
                )
            })
        };
        let alias = required(self, "alias", web_search::MAX_ALIAS_BYTES)?;
        let url = required(self, "url", web_search::MAX_TEMPLATE_BYTES)?;
        let title = required(self, "title", MAX_TITLE_BYTES)?;
        let fallback: Value = object.get("fallback")?;
        let fallback = if fallback.is_undefined() || fallback.is_null() {
            None
        } else {
            match fallback.as_number() {
                Some(position)
                    if position.fract() == 0.0
                        && (0.0..=f64::from(web_search::MAX_FALLBACK)).contains(&position) =>
                {
                    Some(position as u32)
                }
                _ => {
                    return Err(self
                        .invalid(
                            "invalid-field",
                            format!("{field}.fallback"),
                            format!("must be a whole number from 0 to {}", web_search::MAX_FALLBACK),
                        )
                        .hint("Fallbacks are offered lowest first; leave `fallback` out to offer none.")
                        .into());
                }
            }
        };
        let search = WebSearch {
            alias,
            url,
            title,
            fallback,
        };
        search.validate().map_err(|message| {
            let (key, message) = message
                .strip_prefix("`webSearch.")
                .and_then(|rest| rest.split_once("` "))
                .unwrap_or(("", message.as_str()));
            let at = if key.is_empty() {
                field.to_owned()
            } else {
                format!("{field}.{key}")
            };
            self.invalid("invalid-field", at, message.to_owned())
                .hint(WEB_SEARCH_HINT)
        })?;
        Ok(Some(search))
    }

    /// A stored blob's handle, or a copied file's `{ path, handle }`. Only their shape is
    /// checked here: Dupin resolves them within the package's own storage when it draws them.
    fn image(&mut self, object: &Object<'_>, field: &str) -> Result<Option<RowImage>, Failure> {
        let value: Value = object.get("image")?;
        if value.is_undefined() || value.is_null() {
            return Ok(None);
        }
        let image = if value.is_string() {
            let handle = self
                .string(object, field, "image", MAX_IMAGE_HANDLE_BYTES)?
                .unwrap_or_default();
            RowImage::Blob(handle)
        } else if let Some(file) = plain_object(&value) {
            self.only_keys(&file, &["path", "handle"], IMAGE_HINT)
                .map_err(|failure| self.nested(failure, field))?;
            let path = self.string(&file, &format!("{field}.path"), "path", MAX_FILE_BYTES)?;
            let handle = self.string(
                &file,
                &format!("{field}.handle"),
                "handle",
                MAX_IMAGE_HANDLE_BYTES,
            )?;
            let (Some(path), Some(handle)) = (path, handle) else {
                return Err(self
                    .invalid("missing-field", field, "needs both `path` and `handle`")
                    .hint(IMAGE_HINT)
                    .into());
            };
            RowImage::File(CopiedFileImage { path, handle })
        } else {
            return Err(self
                .invalid(
                    "invalid-field",
                    field,
                    format!(
                        "must be a handle or `{{ path, handle }}`, not {}",
                        describe(&value)
                    ),
                )
                .hint(IMAGE_HINT)
                .into());
        };
        row::check_image(&image)
            .map_err(|message| self.invalid("invalid-field", field, message).hint(IMAGE_HINT))?;
        Ok(Some(image))
    }

    /// Milliseconds since the epoch, as `Date.now()` gives them.
    fn time(&self, item: &Object<'_>, field: &str) -> Result<Option<u64>, Failure> {
        let value: Value = item.get("time")?;
        if value.is_undefined() || value.is_null() {
            return Ok(None);
        }
        let valid = value
            .as_number()
            .filter(|time| time.fract() == 0.0 && *time >= 0.0 && *time <= MAX_TIME_MS as f64);
        match valid {
            Some(time) => Ok(Some(time as u64)),
            None => Err(self
                .invalid(
                    "invalid-field",
                    field,
                    format!("must be milliseconds since the epoch, not {}", describe(&value)),
                )
                .hint("Give `Date.now()` or `date.getTime()`.")
                .into()),
        }
    }

    fn openable_url(&self, field: &str, url: &str) -> Result<(), Failure> {
        if row::is_openable(url, self.source) {
            return Ok(());
        }
        let allowed = match self.source {
            // A catalogue reaches no host service, so no calendar link reached it.
            Source::Items => "must be an http or https URL with a host, or a dict:// URL",
            Source::Search => {
                "must be an http or https URL with a host, a dict:// URL, or a calendar event's `link`"
            }
        };
        Err(self
            .invalid("invalid-url", field, allowed)
            .hint("Use a full URL such as `https://example.com/`; other schemes can't be opened.")
            .into())
    }

    /// An item's `aliases` or `keywords`, which `key` names.
    fn words(&mut self, item: &Object<'_>, field: &str, key: &str) -> Result<Vec<String>, Failure> {
        let value: Value = item.get(key)?;
        if value.is_undefined() || value.is_null() {
            return Ok(Vec::new());
        }
        let Some(list) = value.into_array() else {
            return Err(self
                .invalid("invalid-field", field, "must be an array of strings")
                .hint("Give words that should also find the item, such as `[\"docs\", \"manual\"]`.")
                .into());
        };
        if list.len() > MAX_KEYWORDS {
            return Err(self
                .invalid(
                    "invalid-field",
                    field,
                    format!("has more than {MAX_KEYWORDS} {key}"),
                )
                .hint("Keep the few words people would type.")
                .into());
        }
        let mut keywords = Vec::with_capacity(list.len());
        for (index, keyword) in list.iter::<Value>().enumerate() {
            let at = format!("{field}[{index}]");
            let keyword = keyword?;
            let Some(text) = keyword.as_string() else {
                return Err(self
                    .invalid(
                        "invalid-field",
                        at,
                        format!("must be a string, not {}", describe(&keyword)),
                    )
                    .hint("Convert values with `String(value)`.")
                    .into());
            };
            let text = text.to_string()?;
            if text.is_empty() || printable(&text).is_err() {
                return Err(self
                    .invalid("invalid-field", at, "must be one line of text")
                    .hint(format!("Leave empty {key} out."))
                    .into());
            }
            self.count(&at, &text, MAX_KEYWORD_BYTES)?;
            keywords.push(text);
        }
        Ok(keywords)
    }

    fn detail(&mut self, item: &Object<'_>, field: &str) -> Result<Option<RowDetail>, Failure> {
        let value: Value = item.get("detail")?;
        if value.is_undefined() || value.is_null() {
            return Ok(None);
        }
        let Some(detail) = plain_object(&value) else {
            return Err(self
                .invalid(
                    "invalid-field",
                    field,
                    format!("must be an object, not {}", describe(&value)),
                )
                .hint("Give `{ markdown: \"…\", metadata: [{ label: \"…\", text: \"…\" }] }`.")
                .into());
        };
        self.only_keys(
            &detail,
            &["image", "markdown", "metadata"],
            "A detail takes `image`, `markdown` and `metadata`.",
        )
        .map_err(|failure| self.nested(failure, field))?;
        self.detail_parts(&detail, field).map(Some)
    }

    /// A detail's `image`, `markdown` and `metadata`, from `detail` at `field`, which is empty
    /// for a detail view's own.
    fn detail_parts(&mut self, detail: &Object<'_>, field: &str) -> Result<RowDetail, Failure> {
        let image = self.image(detail, &join(field, "image"))?;
        let markdown = self
            .string(detail, &join(field, "markdown"), "markdown", MAX_MARKDOWN_BYTES)?
            .unwrap_or_default();
        let value: Value = detail.get("metadata")?;
        let mut metadata = Vec::new();
        if !value.is_undefined() && !value.is_null() {
            let at = join(field, "metadata");
            let Some(list) = value.into_array() else {
                return Err(self
                    .invalid("invalid-field", at, "must be an array of `{ label, text }`")
                    .hint("Give `[{ label: \"Size\", text: \"2 KB\" }]`.")
                    .into());
            };
            if list.len() > MAX_METADATA {
                return Err(self
                    .invalid(
                        "invalid-field",
                        at,
                        format!("has more than {MAX_METADATA} entries"),
                    )
                    .hint("Move the rest into `markdown`.")
                    .into());
            }
            for (index, entry) in list.iter::<Value>().enumerate() {
                let at = format!("{at}[{index}]");
                let entry = entry?;
                let Some(entry) = plain_object(&entry) else {
                    return Err(self
                        .invalid("invalid-field", at, "must be `{ label, text }`")
                        .hint("Give `{ label: \"Size\", text: \"2 KB\" }`.")
                        .into());
                };
                self.only_keys(
                    &entry,
                    &["label", "text"],
                    "A metadata entry takes `label` and `text`.",
                )
                .map_err(|failure| self.nested(failure, &at))?;
                let label = self.line(&entry, &format!("{at}.label"), "label", MAX_LABEL_BYTES)?;
                let text = self.line(&entry, &format!("{at}.text"), "text", MAX_METADATA_TEXT_BYTES)?;
                let (Some(label), Some(text)) = (label, text) else {
                    return Err(self
                        .invalid("missing-field", at, "needs both `label` and `text`")
                        .hint("Give `{ label: \"Size\", text: \"2 KB\" }`.")
                        .into());
                };
                metadata.push(Metadata { label, text });
            }
        }
        Ok(RowDetail {
            image,
            markdown,
            metadata,
        })
    }

    /// What `views[name]` returned: a list as `search` returns one, a detail or a form.
    pub fn view_answer(&mut self, value: Value<'_>) -> Result<ViewAnswer, Failure> {
        if value.is_array() {
            return Ok(ViewAnswer::List {
                rows: self.list_items(value, "items", None)?,
                empty_text: None,
            });
        }
        let not_a_view = |reader: &Self, value: &Value<'_>| -> Failure {
            Diagnostic::new(
                "invalid-result",
                format!(
                    "the view returned {}, not a list, a detail or a form",
                    describe(value)
                ),
            )
            .file(reader.entry)
            .hint("Return `{ type: \"list\", items: [] }`, `{ type: \"detail\", title, markdown }` or `{ type: \"form\", title, fields, submit }`.")
            .into()
        };
        let Some(view) = plain_object(&value) else {
            return Err(not_a_view(self, &value));
        };
        let kind: Value = view.get("type")?;
        let kind = if kind.is_undefined() {
            None
        } else {
            kind.as_string().map(|kind| kind.to_string()).transpose()?
        };
        match kind.as_deref() {
            None | Some("list") => {
                self.only_keys(
                    &view,
                    &["type", "items", "sections", "emptyText"],
                    "A list takes `items` or `sections`, and optionally `type: \"list\"` and `emptyText`.",
                )?;
                let empty_text = self.line(&view, "emptyText", "emptyText", MAX_EMPTY_TEXT_BYTES)?;
                let items: Value = view.get("items")?;
                let sections: Value = view.get("sections")?;
                let rows = if items.is_undefined() && sections.is_undefined() {
                    Vec::new()
                } else {
                    self.list(&view)?
                };
                Ok(ViewAnswer::List { rows, empty_text })
            }
            Some("detail") => {
                self.only_keys(
                    &view,
                    &["type", "title", "markdown", "metadata", "image", "actions"],
                    "A detail takes `title`, `markdown`, `metadata`, `image` and `actions`.",
                )?;
                let title = self
                    .line(&view, "title", "title", MAX_TITLE_BYTES)?
                    .ok_or_else(|| {
                        self.invalid("missing-field", "title", "is required")
                            .hint("Give the detail a `title`.")
                    })?;
                let detail = self.detail_parts(&view, "")?;
                let mut actions = Vec::new();
                self.actions(&view, "actions", &mut actions)?;
                Ok(ViewAnswer::Detail {
                    title,
                    detail,
                    actions,
                })
            }
            Some("form") => self.form(&view).map(ViewAnswer::Form),
            Some(_) => Err(self
                .invalid(
                    "invalid-field",
                    "type",
                    "must be \"list\", \"detail\" or \"form\"",
                )
                .hint("A view is a list, a detail or a form.")
                .into()),
        }
    }

    fn form(&mut self, view: &Object<'_>) -> Result<Form, Failure> {
        const HINT: &str = "Give `{ type: \"form\", title, fields: [{ type: \"text\", id: \"name\", title: \"Name\" }], submit: \"save\" }`.";
        self.only_keys(
            view,
            &["type", "title", "fields", "submit", "submitTitle"],
            "A form takes `title`, `fields`, `submit` and `submitTitle`.",
        )?;
        let title = self
            .line(view, "title", "title", MAX_TITLE_BYTES)?
            .ok_or_else(|| self.invalid("missing-field", "title", "is required").hint(HINT))?;
        let submit = self
            .string(view, "submit", "submit", row::MAX_HANDLER_BYTES)?
            .ok_or_else(|| {
                self.invalid("missing-field", "submit", "is required")
                    .hint("Name the function of the module's `actions` that takes the form's values.")
            })?;
        row::check_handler(&submit).map_err(|message| {
            self.invalid("invalid-field", "submit", message.replace("`run` must", "must"))
                .hint("Name the function of the module's `actions` that takes the form's values.")
        })?;
        let submit_title = self.line(view, "submitTitle", "submitTitle", MAX_FIELD_TITLE_BYTES)?;
        let fields: Value = view.get("fields")?;
        let Some(list) = fields.into_array() else {
            return Err(self
                .invalid("invalid-field", "fields", "must be an array of fields")
                .hint(HINT)
                .into());
        };
        if list.is_empty() || list.len() > MAX_FIELDS {
            return Err(self
                .invalid(
                    "invalid-field",
                    "fields",
                    format!("must have 1 to {MAX_FIELDS} fields"),
                )
                .hint(HINT)
                .into());
        }
        let mut fields = Vec::with_capacity(list.len());
        for (index, field) in list.iter::<Value>().enumerate() {
            fields.push(self.field(&format!("fields[{index}]"), field?)?);
        }
        let form = Form {
            title,
            fields,
            submit,
            submit_title,
        };
        form.validate()
            .map_err(|message| self.invalid("invalid-form", "fields", message).hint(HINT))?;
        Ok(form)
    }

    fn field(&mut self, at: &str, value: Value<'_>) -> Result<Field, Failure> {
        const HINT: &str = "A field is `{ type, id, title }` with `type` \"text\", \"textarea\", \"checkbox\" or \"select\"; text fields take `required`, `default` and `placeholder`, a checkbox `default` and a select `options` and `default`.";
        let Some(field) = plain_object(&value) else {
            return Err(self
                .invalid("invalid-field", at, "must be an object")
                .hint(HINT)
                .into());
        };
        let kind: Value = field.get("type")?;
        let kind = kind.as_string().map(|kind| kind.to_string()).transpose()?;
        let allowed: &[&str] = match kind.as_deref() {
            Some("text" | "textarea") => &["type", "id", "title", "required", "default", "placeholder"],
            Some("checkbox") => &["type", "id", "title", "default"],
            Some("select") => &["type", "id", "title", "options", "default"],
            _ => {
                return Err(self
                    .invalid(
                        "invalid-field",
                        format!("{at}.type"),
                        "must be \"text\", \"textarea\", \"checkbox\" or \"select\"",
                    )
                    .hint(HINT)
                    .into());
            }
        };
        self.only_keys(&field, allowed, HINT)
            .map_err(|failure| self.nested(failure, at))?;
        let id = self
            .string(&field, &format!("{at}.id"), "id", MAX_FIELD_ID_BYTES)?
            .ok_or_else(|| {
                self.invalid("missing-field", format!("{at}.id"), "is required")
                    .hint(HINT)
            })?;
        view::check_field_id(&id).map_err(|message| {
            self.invalid("invalid-field", format!("{at}.id"), message)
                .hint("Name the field as its value is named in what the submit receives, such as `title`.")
        })?;
        let title = self
            .line(&field, &format!("{at}.title"), "title", MAX_FIELD_TITLE_BYTES)?
            .ok_or_else(|| {
                self.invalid("missing-field", format!("{at}.title"), "is required")
                    .hint("Give the field a `title`, which Dupin shows beside it.")
            })?;
        let kind = match kind.as_deref().expect("matched above") {
            "text" | "textarea" => {
                let multiline = kind.as_deref() == Some("textarea");
                let required = self
                    .boolean(&field, "required")
                    .map_err(|failure| self.nested(failure, at))?
                    .unwrap_or(false);
                let cap = if multiline {
                    MAX_TEXTAREA_BYTES
                } else {
                    MAX_TEXT_FIELD_BYTES
                };
                let default = self
                    .uncounted_text(&field, &format!("{at}.default"), cap)?
                    .unwrap_or_default();
                if !multiline && printable(&default).is_err() {
                    return Err(self
                        .invalid("invalid-field", format!("{at}.default"), "must be one line")
                        .hint("Make the field a `textarea` to take several lines.")
                        .into());
                }
                self.count(&format!("{at}.default"), &default, cap)?;
                let placeholder = self.line(
                    &field,
                    &format!("{at}.placeholder"),
                    "placeholder",
                    MAX_FIELD_TITLE_BYTES,
                )?;
                if multiline {
                    FieldKind::Textarea {
                        required,
                        default,
                        placeholder,
                    }
                } else {
                    FieldKind::Text {
                        required,
                        default,
                        placeholder,
                    }
                }
            }
            "checkbox" => FieldKind::Checkbox {
                default: self
                    .boolean(&field, "default")
                    .map_err(|failure| self.nested(failure, at))?
                    .unwrap_or(false),
            },
            _ => {
                let options: Value = field.get("options")?;
                let Some(list) = options.into_array() else {
                    return Err(self
                        .invalid(
                            "invalid-field",
                            format!("{at}.options"),
                            "must be an array of `{ value, title }`",
                        )
                        .hint("Give `options: [{ value: \"red\", title: \"Red\" }]`.")
                        .into());
                };
                if list.is_empty() || list.len() > MAX_OPTIONS {
                    return Err(self
                        .invalid(
                            "invalid-field",
                            format!("{at}.options"),
                            format!("must have 1 to {MAX_OPTIONS} options"),
                        )
                        .into());
                }
                let mut options = Vec::with_capacity(list.len());
                for (index, option) in list.iter::<Value>().enumerate() {
                    let at = format!("{at}.options[{index}]");
                    let option = option?;
                    let Some(option) = plain_object(&option) else {
                        return Err(self
                            .invalid("invalid-field", &at, "must be `{ value, title }`")
                            .into());
                    };
                    self.only_keys(
                        &option,
                        &["value", "title"],
                        "An option takes `value` and `title`.",
                    )
                    .map_err(|failure| self.nested(failure, &at))?;
                    let value = self.line(&option, &format!("{at}.value"), "value", MAX_OPTION_BYTES)?;
                    let title = self.line(&option, &format!("{at}.title"), "title", MAX_OPTION_BYTES)?;
                    let (Some(value), Some(title)) = (value, title) else {
                        return Err(self
                            .invalid("missing-field", &at, "needs both `value` and `title`")
                            .into());
                    };
                    options.push(SelectOption { value, title });
                }
                let default = self
                    .line(&field, &format!("{at}.default"), "default", MAX_OPTION_BYTES)?
                    .unwrap_or_else(|| options[0].value.clone());
                FieldKind::Select { options, default }
            }
        };
        Ok(Field { id, title, kind })
    }

    /// An optional string field of up to `cap` bytes, which may hold several lines.
    fn uncounted_text(
        &self,
        object: &Object<'_>,
        field: &str,
        cap: usize,
    ) -> Result<Option<String>, Failure> {
        let value: Value = object.get("default")?;
        if value.is_undefined() || value.is_null() {
            return Ok(None);
        }
        let Some(text) = value.as_string() else {
            return Err(self
                .invalid(
                    "invalid-field",
                    field,
                    format!("must be a string, not {}", describe(&value)),
                )
                .into());
        };
        let text = text.to_string()?;
        self.capped(field, &text, cap)?;
        Ok(Some(text))
    }

    /// Reads the item's own `actions` after the ones `url` and `text` made.
    fn actions(
        &mut self,
        item: &Object<'_>,
        field: &str,
        actions: &mut Vec<RowAction>,
    ) -> Result<(), Failure> {
        let value: Value = item.get("actions")?;
        if value.is_undefined() || value.is_null() {
            return Ok(());
        }
        let Some(list) = value.into_array() else {
            return Err(self
                .invalid("invalid-field", field, "must be an array of actions")
                .hint("Give `[{ title: \"Copy\", copy: \"text\" }]`.")
                .into());
        };
        if actions.len() + list.len() > MAX_ACTIONS {
            return Err(self
                .invalid(
                    "invalid-field",
                    field,
                    format!("gives the item more than {MAX_ACTIONS} actions, counting `url` and `text`"),
                )
                .hint("Keep the actions people use most.")
                .into());
        }
        for (index, action) in list.iter::<Value>().enumerate() {
            let at = format!("{field}[{index}]");
            let action = action?;
            let Some(action) = plain_object(&action) else {
                return Err(self
                    .invalid("invalid-field", &at, "must be an object")
                    .hint("Give `{ title: \"Copy\", copy: \"text\" }`.")
                    .into());
            };
            let mut kinds = Vec::new();
            for key in action.keys::<String>() {
                let key = key?;
                match key.as_str() {
                    "title" | "data" | "alternate" | "confirm" | "needsFile" | "template" => {}
                    "copy" | "paste" | "open" | "reveal" | "run" | "push" => kinds.push(key),
                    _ => {
                        return Err(self
                            .invalid("unknown-field", format!("{at}.{key}"), "is not an action field")
                            .hint("An action takes `title` and one of `copy`, `paste`, `open`, `reveal`, `run` or `push`, with `data`, `alternate` and `confirm` for `run` and `template` for `copy` or `paste`.")
                            .into());
                    }
                }
            }
            let title = self
                .line(&action, &format!("{at}.title"), "title", MAX_ACTION_TITLE_BYTES)?
                .ok_or_else(|| {
                    self.invalid("missing-field", format!("{at}.title"), "is required")
                        .hint("Name what the action does, such as `Copy Link`.")
                })?;
            let [kind] = kinds.as_slice() else {
                return Err(self
                    .invalid("invalid-field", &at, "must have exactly one of `copy`, `paste`, `open`, `reveal`, `run` or `push`")
                    .hint("Give `{ title: \"Open\", open: \"https://…\" }` or `{ title: \"Delete\", run: \"remove\" }`.")
                    .into());
            };
            let data: Value = action.get("data")?;
            for key in ["data", "alternate", "confirm", "needsFile"] {
                let value: Value = action.get(key)?;
                if *kind != "run" && !value.is_undefined() {
                    return Err(self
                        .invalid("unknown-field", format!("{at}.{key}"), "only goes with `run`")
                        .hint(format!("Leave `{key}` out, or make the action `run` a handler."))
                        .into());
                }
            }
            let alternate = self
                .boolean(&action, "alternate")
                .map_err(|failure| self.nested(failure, &at))?
                .unwrap_or(false);
            if alternate && actions.iter().any(|action: &RowAction| action.alternate) {
                return Err(self
                    .invalid(
                        "invalid-field",
                        format!("{at}.alternate"),
                        "is set on another action too",
                    )
                    .hint("Only one action is ⌘Enter.")
                    .into());
            }
            let template = self.template(&action, &at, kind)?;
            let confirm = self.line(
                &action,
                &format!("{at}.confirm"),
                "confirm",
                MAX_ACTION_TITLE_BYTES,
            )?;
            let needs_file = self
                .boolean(&action, "needsFile")
                .map_err(|failure| self.nested(failure, &at))?
                .unwrap_or(false);
            let effect = match kind.as_str() {
                "copy" => RowEffect::Copy {
                    text: self
                        .string(&action, &format!("{at}.copy"), "copy", MAX_ACTION_TEXT_BYTES)?
                        .expect("present"),
                },
                "paste" => RowEffect::Paste {
                    text: self
                        .string(&action, &format!("{at}.paste"), "paste", MAX_ACTION_TEXT_BYTES)?
                        .expect("present"),
                },
                "open" => {
                    let field = format!("{at}.open");
                    let url = self
                        .string(&action, &field, "open", MAX_URL_BYTES)?
                        .expect("present");
                    self.openable_url(&field, &url)?;
                    RowEffect::OpenUrl { url }
                }
                "reveal" => {
                    let field = format!("{at}.reveal");
                    let path = self
                        .string(&action, &field, "reveal", row::MAX_FILE_BYTES)?
                        .expect("present");
                    row::check_file(&path).map_err(|message| {
                        self.invalid("invalid-field", &field, message.replace("`file` must", "must"))
                            .hint("Give the full path of the file to show in Finder.")
                    })?;
                    RowEffect::Reveal { path }
                }
                "push" => self.push(&action, &format!("{at}.push"))?,
                _ => {
                    let field = format!("{at}.run");
                    let handler = self
                        .string(&action, &field, "run", row::MAX_HANDLER_BYTES)?
                        .expect("present");
                    row::check_handler(&handler).map_err(|message| {
                        self.invalid("invalid-field", &field, message.replace("`run` must", "must"))
                            .hint("Export `actions` with a function of that name.")
                    })?;
                    let data = self.data(data, &format!("{at}.data"))?;
                    RowEffect::Run { handler, data }
                }
            };
            actions.push(RowAction {
                title,
                effect,
                template,
                alternate,
                confirm,
                needs_file,
            });
        }
        Ok(())
    }

    /// A `push` action's `{ view, state }`.
    fn push(&mut self, action: &Object<'_>, field: &str) -> Result<RowEffect, Failure> {
        const HINT: &str = "Give `push: { view: \"details\", state: { id: 7 } }`, naming a function the module's `views` export.";
        let value: Value = action.get("push")?;
        let Some(push) = plain_object(&value) else {
            return Err(self
                .invalid("invalid-field", field, "must be `{ view, state }`")
                .hint(HINT)
                .into());
        };
        self.only_keys(&push, &["view", "state"], HINT)
            .map_err(|failure| self.nested(failure, field))?;
        let at = format!("{field}.view");
        let view = self
            .string(&push, &at, "view", row::MAX_HANDLER_BYTES)?
            .ok_or_else(|| self.invalid("missing-field", &at, "is required").hint(HINT))?;
        row::check_name(&view, "view").map_err(|message| {
            self.invalid("invalid-field", &at, message.replace("`view` must", "must"))
                .hint(HINT)
        })?;
        let state: Value = push.get("state")?;
        let state = if state.is_undefined() {
            None
        } else {
            let at = format!("{field}.state");
            let Some(state) = json(&state) else {
                return Err(self
                    .invalid(
                        "invalid-field",
                        &at,
                        "must be JSON: strings, numbers, booleans, null, arrays or objects",
                    )
                    .hint("Keep plain data in a page's state, such as the id of what it shows.")
                    .into());
            };
            row::check_state(&state).map_err(|message| {
                self.invalid("invalid-field", &at, message.replace("`state` ", ""))
                    .hint("Keep an id in the state and look the rest up in the view.")
            })?;
            self.bytes += state.to_string().len();
            self.check_total(&at)?;
            Some(state)
        };
        Ok(RowEffect::Push { view, state })
    }

    /// An action's `template`: a boolean, and only on a `copy` or `paste`.
    fn template(&mut self, action: &Object<'_>, at: &str, kind: &str) -> Result<bool, Failure> {
        let field = format!("{at}.template");
        let value: Value = action.get("template")?;
        if value.is_undefined() {
            return Ok(false);
        }
        let Some(template) = value.as_bool() else {
            return Err(self
                .invalid("invalid-field", &field, "must be a boolean")
                .hint("Give `template: true` to have Dupin fill `{date}`, `{time}`, `{datetime}` and `{clipboard}`.")
                .into());
        };
        if template && kind != "copy" && kind != "paste" {
            return Err(self
                .invalid("unknown-field", &field, "only goes with `copy` or `paste`")
                .hint("Leave `template` out, or make the action `copy` or `paste` its text.")
                .into());
        }
        Ok(template)
    }

    fn data(&mut self, value: Value<'_>, field: &str) -> Result<Option<serde_json::Value>, Failure> {
        if value.is_undefined() {
            return Ok(None);
        }
        let ctx = value.ctx().clone();
        let json = ctx.json_stringify(value)?;
        let parsed = json
            .map(|json| json.to_string())
            .transpose()?
            .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok());
        let Some(data) = parsed else {
            return Err(self
                .invalid(
                    "invalid-field",
                    field,
                    "must be JSON: strings, numbers, booleans, null, arrays or objects",
                )
                .hint("Pass plain data, such as the item's id; functions and `undefined` don't survive.")
                .into());
        };
        row::check_data(&data).map_err(|message| {
            self.invalid("invalid-field", field, message.replace("`data` ", ""))
                .hint("Pass an id and look the rest up in the handler.")
        })?;
        self.bytes += data.to_string().len();
        self.check_total(field)?;
        Ok(Some(data))
    }

    /// An optional string that must be one line.
    fn line(
        &mut self,
        object: &Object<'_>,
        field: &str,
        key: &str,
        cap: usize,
    ) -> Result<Option<String>, Failure> {
        let value = self.string(object, field, key, cap)?;
        if let Some(value) = &value {
            printable(value).map_err(|()| {
                self.invalid(
                    "invalid-field",
                    field,
                    "must be one line without control characters",
                )
                .hint("Shorten it to one line.")
            })?;
        }
        Ok(value)
    }

    /// An optional answer: one line, or two, such as an amount and where its rate came from.
    fn answer(&mut self, object: &Object<'_>, field: &str) -> Result<Option<String>, Failure> {
        let value = self.string(object, field, "answer", MAX_TITLE_BYTES)?;
        if let Some(value) = &value {
            let (first, second) = value.split_once('\n').unwrap_or((value, ""));
            printable(first)
                .and_then(|()| printable(second))
                .and_then(|()| {
                    if first.is_empty() || value.ends_with('\n') {
                        Err(())
                    } else {
                        Ok(())
                    }
                })
                .map_err(|()| {
                    self.invalid(
                        "invalid-field",
                        field,
                        "must be one or two lines without other control characters",
                    )
                    .hint("Shorten it to two lines.")
                })?;
        }
        Ok(value)
    }

    /// A failure from a nested object, with the field named from the item.
    fn nested(&self, failure: Failure, at: &str) -> Failure {
        match failure {
            Failure::Report(diagnostic) => {
                let field = diagnostic.field.clone().unwrap_or_default();
                Failure::Report(diagnostic.field(format!("{at}.{field}")))
            }
            failure => failure,
        }
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
        let string = self.uncounted(item, field, key, cap)?;
        if let Some(string) = &string {
            self.bytes += string.len();
            self.check_total(field)?;
        }
        Ok(string)
    }

    /// An optional string field, as [`Self::string`] reads it, without counting it.
    fn uncounted(
        &self,
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
        if string.is_empty() && !matches!(key, "subtitle" | "markdown") {
            return Err(self
                .invalid("invalid-field", field, "is empty")
                .hint(format!("Leave `{key}` out instead of giving an empty string."))
                .into());
        }
        self.capped(field, &string, cap)?;
        Ok(Some(string))
    }

    fn count(&mut self, field: &str, string: &str, cap: usize) -> Result<(), Failure> {
        self.capped(field, string, cap)?;
        self.bytes += string.len();
        self.check_total(field)
    }

    fn capped(&self, field: &str, string: &str, cap: usize) -> Result<(), Failure> {
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
        Ok(())
    }

    fn check_total(&self, field: &str) -> Result<(), Failure> {
        if self.bytes <= self.max_bytes() {
            return Ok(());
        }
        let hint = match self.source {
            Source::Search => "Return fewer or shorter items.",
            Source::Items => "Answer with fewer items and `more: true`; Dupin asks again for the rest.",
        };
        Err(Diagnostic::new(
            "output-too-large",
            format!(
                "{} returned more than {} KiB of text",
                self.name,
                self.max_bytes() / 1024
            ),
        )
        .file(self.entry)
        .field(field)
        .hint(hint)
        .into())
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
