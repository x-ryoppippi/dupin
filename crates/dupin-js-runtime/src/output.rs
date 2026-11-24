//! Reads what `search` or `items` returned into rows, checking every value against the limits
//! in [`crate::row`] and naming the field at fault.

use std::collections::HashSet;

use dupin_plugin_protocol::{MAX_RESPONSE_BYTES, MAX_URL_BYTES};
use rquickjs::{Array, Object, Value};
use serde::{Deserialize, Serialize};

use crate::row::{
    self, CataloguePage, MAX_ACTION_TITLE_BYTES, MAX_ACTIONS, MAX_CURSOR_BYTES, MAX_ID_BYTES,
    MAX_KEYWORD_BYTES, MAX_KEYWORDS, MAX_LABEL_BYTES, MAX_MARKDOWN_BYTES, MAX_METADATA,
    MAX_METADATA_TEXT_BYTES, MAX_PAGE_BYTES, MAX_PAGE_ROWS, MAX_SECTION_BYTES, MAX_SUBTITLE_BYTES,
    MAX_TEXT_BYTES, MAX_TITLE_BYTES, MAX_TTL_SECONDS, Metadata, PageChange, Row, RowAction, RowDetail,
    RowEffect,
};
use crate::{Diagnostic, Failure, MAX_ROWS};

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

const ITEM_FIELDS: [&str; 9] = [
    "id", "title", "subtitle", "text", "url", "icon", "section", "detail", "actions",
];
const ITEM_FIELDS_HINT: &str =
    "Items take `id`, `title`, `subtitle`, `text`, `url`, `icon`, `section`, `detail` and `actions`.";
const CATALOGUE_ITEM_FIELDS_HINT: &str = "Items take `id`, `title`, `subtitle`, `keywords`, `text`, `url`, `icon`, `section`, `detail` and `actions`.";

/// Which export answered, which decides the fields and limits that apply.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Search,
    Items,
}

impl Source {
    fn name(self) -> &'static str {
        match self {
            Source::Search => "search",
            Source::Items => "items",
        }
    }
}

pub(crate) struct Reader<'a> {
    pub entry: &'a str,
    pub limit: usize,
    source: Source,
    bytes: usize,
}

impl<'a> Reader<'a> {
    pub fn search(entry: &'a str, limit: usize) -> Self {
        Self {
            entry,
            limit,
            source: Source::Search,
            bytes: 0,
        }
    }

    pub fn items(entry: &'a str) -> Self {
        Self {
            entry,
            limit: MAX_PAGE_ROWS,
            source: Source::Items,
            bytes: 0,
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
        let cursor = self.string(&answer, "cursor", "cursor", MAX_CURSOR_BYTES)?;
        let ttl = self.ttl(&answer)?;
        let change = if update {
            self.only_keys(
                &answer,
                &["type", "append", "replace", "remove", "ttl", "cursor", "more"],
                "An update takes `append`, `replace` and `remove`, and optionally `ttl`, `cursor` and `more`.",
            )?;
            let append = self.optional_items(&answer, "append")?;
            let replace = self.optional_items(&answer, "replace")?;
            if append.len() + replace.len() > MAX_PAGE_ROWS {
                return Err(self.too_many_rows(append.len() + replace.len()));
            }
            let mut ids = HashSet::new();
            for (field, row) in append
                .iter()
                .enumerate()
                .map(|(i, row)| (format!("append[{i}].id"), row))
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
        let name = self.source.name();
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
        let name = self.source.name();
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
            let catalogue_only = key == "keywords" && self.source == Source::Items;
            if !ITEM_FIELDS.contains(&key.as_str()) && !catalogue_only {
                let hint = match self.source {
                    Source::Search if key == "keywords" => {
                        "`keywords` help Dupin match items it holds; export `items` to use them."
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
        let keywords = self.keywords(&item, &field("keywords"))?;
        let icon = self.string(&item, &field("icon"), "icon", row::MAX_ICON_BYTES)?;
        if let Some(icon) = &icon {
            row::check_icon(icon).map_err(|message| {
                self.invalid("invalid-field", field("icon"), message.replace("`icon` must", "must"))
                    .hint("Ship the image in the package and name it relative to `package.json`, such as `assets/icon.png`.")
            })?;
        }
        let section = self.line(&item, &field("section"), "section", MAX_SECTION_BYTES)?;
        let detail = self.detail(&item, &field("detail"))?;
        let mut actions = Vec::new();
        if let Some(url) = self.string(&item, &field("url"), "url", MAX_URL_BYTES)? {
            self.openable_url(&field("url"), &url)?;
            actions.push(RowAction {
                title: "Open URL".into(),
                effect: RowEffect::OpenUrl { url },
            });
        }
        if let Some(text) = self.string(&item, &field("text"), "text", MAX_TEXT_BYTES)? {
            actions.push(RowAction {
                title: "Copy".into(),
                effect: RowEffect::Copy { text },
            });
        }
        self.actions(&item, &field("actions"), &mut actions)?;
        Ok(Row {
            id,
            title,
            subtitle,
            keywords,
            icon,
            section,
            detail,
            actions,
        })
    }

    fn openable_url(&self, field: &str, url: &str) -> Result<(), Failure> {
        if row::is_openable_url(url) {
            return Ok(());
        }
        Err(self
            .invalid(
                "invalid-url",
                field,
                "must be an http or https URL with a host, or a dict:// URL",
            )
            .hint("Use a full URL such as `https://example.com/`; other schemes can't be opened.")
            .into())
    }

    fn keywords(&mut self, item: &Object<'_>, field: &str) -> Result<Vec<String>, Failure> {
        let value: Value = item.get("keywords")?;
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
                    format!("has more than {MAX_KEYWORDS} keywords"),
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
                    .hint("Leave empty keywords out.")
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
            &["markdown", "metadata"],
            "A detail takes `markdown` and `metadata`.",
        )
        .map_err(|failure| self.nested(failure, field))?;
        let markdown = self
            .string(
                &detail,
                &format!("{field}.markdown"),
                "markdown",
                MAX_MARKDOWN_BYTES,
            )?
            .unwrap_or_default();
        let value: Value = detail.get("metadata")?;
        let mut metadata = Vec::new();
        if !value.is_undefined() && !value.is_null() {
            let at = format!("{field}.metadata");
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
        Ok(Some(RowDetail { markdown, metadata }))
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
                    "title" | "data" => {}
                    "copy" | "open" | "run" => kinds.push(key),
                    _ => {
                        return Err(self
                            .invalid("unknown-field", format!("{at}.{key}"), "is not an action field")
                            .hint("An action takes `title` and one of `copy`, `open` or `run`, with `data` for `run`.")
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
                    .invalid("invalid-field", &at, "must have exactly one of `copy`, `open` or `run`")
                    .hint("Give `{ title: \"Open\", open: \"https://…\" }` or `{ title: \"Delete\", run: \"remove\" }`.")
                    .into());
            };
            let data: Value = action.get("data")?;
            if *kind != "run" && !data.is_undefined() {
                return Err(self
                    .invalid("unknown-field", format!("{at}.data"), "only goes with `run`")
                    .hint("Leave `data` out, or make the action `run` a handler.")
                    .into());
            }
            let effect = match kind.as_str() {
                "copy" => RowEffect::Copy {
                    text: self
                        .string(&action, &format!("{at}.copy"), "copy", MAX_TEXT_BYTES)?
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
            actions.push(RowAction { title, effect });
        }
        Ok(())
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
        self.count(field, &string, cap)?;
        Ok(Some(string))
    }

    fn count(&mut self, field: &str, string: &str, cap: usize) -> Result<(), Failure> {
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
        self.check_total(field)
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
                self.source.name(),
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
