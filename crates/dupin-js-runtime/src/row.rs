//! The rows an extension answers with, after the reader has checked them, and the same checks
//! again for rows that crossed from the extension host, which Dupin can't trust.

use dupin_plugin_protocol::{MAX_URL_BYTES, is_web_url};
use serde::{Deserialize, Serialize};

pub(crate) const MAX_TITLE_BYTES: usize = 512;
pub(crate) const MAX_SUBTITLE_BYTES: usize = 2048;
pub(crate) const MAX_ID_BYTES: usize = 128;
pub(crate) const MAX_TEXT_BYTES: usize = 16 * 1024;
pub(crate) const MAX_KEYWORDS: usize = 16;
pub(crate) const MAX_KEYWORD_BYTES: usize = 64;
/// As much text as a clipboard change carries, so a history can be found by all of it.
pub(crate) const MAX_MATCH_TEXT_BYTES: usize = 64 * 1024;
pub(crate) const MAX_FILE_BYTES: usize = 4096;
/// Milliseconds since the epoch that JavaScript's `Date` can hold.
pub(crate) const MAX_TIME_MS: u64 = 8_640_000_000_000_000;
pub(crate) const MAX_SECTION_BYTES: usize = 128;
pub(crate) const MAX_ICON_BYTES: usize = 256;
/// Above the handles storage and the clipboard watch hand out, which are 64 hex digits.
pub(crate) const MAX_IMAGE_HANDLE_BYTES: usize = 128;
/// Image formats the launcher draws as a row's icon.
const ICON_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "svg", "pdf", "icns"];
/// What the launcher's Markdown renderer reads before it cuts the rest.
pub(crate) const MAX_MARKDOWN_BYTES: usize = 32 * 1024;
pub(crate) const MAX_METADATA: usize = 12;
pub(crate) const MAX_LABEL_BYTES: usize = 64;
pub(crate) const MAX_METADATA_TEXT_BYTES: usize = 512;
/// Every action of a row, counting the Open URL and Copy that `url` and `text` add.
pub(crate) const MAX_ACTIONS: usize = 8;
pub(crate) const MAX_ACTION_TITLE_BYTES: usize = 512;
pub(crate) const MAX_HANDLER_BYTES: usize = 64;
const MAX_DATA_BYTES: usize = 4096;
const MAX_DATA_DEPTH: usize = 16;

/// Rows one `items()` answer may add or replace, and IDs it may remove.
pub const MAX_PAGE_ROWS: usize = 500;
/// Text in one `items()` answer, the largest any answer carries.
pub const MAX_PAGE_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_CURSOR_BYTES: usize = 256;
/// A day: a catalogue that changes less often can wait for an action to invalidate it.
pub(crate) const MAX_TTL_SECONDS: u32 = 24 * 60 * 60;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subtitle: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    /// More text a catalogue that matches words finds the row by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_text: Option<String>,
    /// Milliseconds since the epoch the row is from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<u64>,
    /// An absolute path whose Finder icon the row shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// A package-relative image path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// An image the package keeps, drawn scaled down in place of `icon` and `file`'s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<RowImage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<RowDetail>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<RowAction>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowDetail {
    /// Shown large above the metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<RowImage>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub markdown: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metadata: Vec<Metadata>,
}

/// An image a row names by an opaque handle, never by its bytes. Dupin resolves it only within
/// the package's own storage, when it draws the row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RowImage {
    /// A blob stored with one of the package's records.
    Blob(String),
    /// A file a clipboard watch delivered to the package, with the handle it came with.
    File(CopiedFileImage),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CopiedFileImage {
    pub path: String,
    pub handle: String,
}

impl RowImage {
    fn text_bytes(&self) -> usize {
        match self {
            RowImage::Blob(handle) => handle.len(),
            RowImage::File(file) => file.path.len() + file.handle.len(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub label: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowAction {
    pub title: String,
    pub effect: RowEffect,
    /// ⌘Enter runs it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub alternate: bool,
    /// What Dupin asks before it runs, with the action's title as the button.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<String>,
    /// It acts on the row's `file`, so it isn't offered while the file is gone.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub needs_file: bool,
}

impl RowAction {
    /// An action Enter or the action list runs, without asking first.
    pub fn new(title: impl Into<String>, effect: RowEffect) -> Self {
        Self {
            title: title.into(),
            effect,
            alternate: false,
            confirm: None,
            needs_file: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RowEffect {
    Copy {
        text: String,
    },
    /// Pastes the text into the app in front of Dupin, which copying alone leaves to the user.
    Paste {
        text: String,
    },
    OpenUrl {
        url: String,
    },
    /// Shows a file selected in Finder; it opens nothing.
    Reveal {
        path: String,
    },
    /// Calls `actions[handler](data)` in the package.
    Run {
        handler: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
    },
}

/// What one `items()` call answered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CataloguePage {
    pub change: PageChange,
    /// Seconds until the catalogue should be asked again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl: Option<u32>,
    /// Handed back to `items()` with the next call, so it can answer with only what changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// Ask again at once, with `cursor`, for the next part.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub more: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PageChange {
    /// The whole catalogue, or its first part when `more` is set.
    Reset { rows: Vec<Row> },
    Update {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        prepend: Vec<Row>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        append: Vec<Row>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        replace: Vec<Row>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        remove: Vec<String>,
    },
}

/// Why the host is calling `items()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// Nothing is held: the first use, or after a failure or a reload.
    Load,
    /// The previous answer said there is more.
    More,
    Expired,
    Invalidated,
}

impl Reason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Reason::Load => "load",
            Reason::More => "more",
            Reason::Expired => "expired",
            Reason::Invalidated => "invalidated",
        }
    }
}

/// Which export answered, which decides the fields and limits that apply: `keywords` only mean
/// something to a catalogue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Search,
    Items,
}

impl Source {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Source::Search => "search",
            Source::Items => "items",
        }
    }
}

impl Row {
    /// Checks every limit the reader enforced, for rows from another process.
    pub fn validate(&self, source: Source) -> Result<(), String> {
        let line = |name: &str, value: &str, cap: usize| -> Result<(), String> {
            if value.is_empty() || value.len() > cap || value.chars().any(char::is_control) {
                Err(format!("`{name}` must be one line of 1 to {cap} bytes"))
            } else {
                Ok(())
            }
        };
        line("id", &self.id, MAX_ID_BYTES)?;
        line("title", &self.title, MAX_TITLE_BYTES)?;
        if self.subtitle.len() > MAX_SUBTITLE_BYTES {
            return Err("`subtitle` is too long".into());
        }
        if source == Source::Search
            && (!self.keywords.is_empty()
                || self.match_text.is_some()
                || self.time.is_some()
                || self.file.is_some())
        {
            return Err("`keywords`, `matchText`, `time` and `file` belong to items()".into());
        }
        if self
            .match_text
            .as_ref()
            .is_some_and(|text| text.is_empty() || text.len() > MAX_MATCH_TEXT_BYTES)
        {
            return Err("`matchText` must be 1 byte to 64 KiB".into());
        }
        if self.time.is_some_and(|time| time > MAX_TIME_MS) {
            return Err("`time` is out of range".into());
        }
        if let Some(file) = &self.file {
            check_file(file)?;
        }
        if self.keywords.len() > MAX_KEYWORDS {
            return Err("too many `keywords`".into());
        }
        for keyword in &self.keywords {
            line("keywords", keyword, MAX_KEYWORD_BYTES)?;
        }
        if let Some(icon) = &self.icon {
            check_icon(icon)?;
        }
        if let Some(image) = &self.image {
            check_image(image).map_err(|message| format!("`image` {message}"))?;
        }
        if let Some(section) = &self.section {
            line("section", section, MAX_SECTION_BYTES)?;
        }
        if let Some(detail) = &self.detail {
            if let Some(image) = &detail.image {
                check_image(image).map_err(|message| format!("`detail.image` {message}"))?;
            }
            if detail.markdown.len() > MAX_MARKDOWN_BYTES {
                return Err("`detail.markdown` is too long".into());
            }
            if detail.metadata.len() > MAX_METADATA {
                return Err("too many `detail.metadata` entries".into());
            }
            for entry in &detail.metadata {
                line("detail.metadata.label", &entry.label, MAX_LABEL_BYTES)?;
                line("detail.metadata.text", &entry.text, MAX_METADATA_TEXT_BYTES)?;
            }
        }
        if self.actions.len() > MAX_ACTIONS {
            return Err("too many `actions`".into());
        }
        if self.actions.iter().filter(|action| action.alternate).count() > 1 {
            return Err("more than one `alternate` action".into());
        }
        for action in &self.actions {
            line("actions.title", &action.title, MAX_ACTION_TITLE_BYTES)?;
            if (action.alternate || action.confirm.is_some() || action.needs_file)
                && !matches!(action.effect, RowEffect::Run { .. })
            {
                return Err("only a `run` action is `alternate`, asks to `confirm` or `needsFile`".into());
            }
            if let Some(confirm) = &action.confirm {
                line("actions.confirm", confirm, MAX_ACTION_TITLE_BYTES)?;
            }
            match &action.effect {
                RowEffect::Copy { text } if text.is_empty() || text.len() > MAX_TEXT_BYTES => {
                    return Err("a copy action's text must be 1 byte to 16 KiB".into());
                }
                RowEffect::Paste { text } if text.is_empty() || text.len() > MAX_TEXT_BYTES => {
                    return Err("a paste action's text must be 1 byte to 16 KiB".into());
                }
                RowEffect::OpenUrl { url } if !is_openable(url, source) => {
                    return Err(match source {
                        Source::Items => "a catalogue's open action needs an http, https or dict URL",
                        Source::Search => {
                            "an open action needs an http, https or dict URL, or a calendar event's link"
                        }
                    }
                    .into());
                }
                RowEffect::Run { handler, data } => {
                    check_handler(handler)?;
                    if let Some(data) = data {
                        check_data(data)?;
                    }
                }
                RowEffect::Reveal { path } => check_file(path)?,
                RowEffect::Copy { .. } | RowEffect::Paste { .. } | RowEffect::OpenUrl { .. } => {}
            }
        }
        Ok(())
    }

    /// The text the row carries, counted toward an answer's limit.
    pub fn text_bytes(&self) -> usize {
        let detail = self.detail.as_ref().map_or(0, |detail| {
            detail.image.as_ref().map_or(0, RowImage::text_bytes)
                + detail.markdown.len()
                + detail
                    .metadata
                    .iter()
                    .map(|entry| entry.label.len() + entry.text.len())
                    .sum::<usize>()
        });
        let actions: usize = self
            .actions
            .iter()
            .map(|action| {
                action.title.len()
                    + action.confirm.as_ref().map_or(0, String::len)
                    + match &action.effect {
                        RowEffect::Copy { text } | RowEffect::Paste { text } => text.len(),
                        RowEffect::OpenUrl { url } => url.len(),
                        RowEffect::Reveal { path } => path.len(),
                        RowEffect::Run { handler, data } => {
                            handler.len() + data.as_ref().map_or(0, |data| data.to_string().len())
                        }
                    }
            })
            .sum();
        self.id.len()
            + self.title.len()
            + self.subtitle.len()
            + self.keywords.iter().map(String::len).sum::<usize>()
            + self.match_text.as_ref().map_or(0, String::len)
            + self.file.as_ref().map_or(0, String::len)
            + self.icon.as_ref().map_or(0, String::len)
            + self.image.as_ref().map_or(0, RowImage::text_bytes)
            + self.section.as_ref().map_or(0, String::len)
            + detail
            + actions
    }
}

impl CataloguePage {
    /// Checks the page's own limits and each of its rows.
    pub fn validate(&self) -> Result<(), String> {
        let (rows, removed): (Vec<&Row>, &[String]) = match &self.change {
            PageChange::Reset { rows } => (rows.iter().collect(), &[]),
            PageChange::Update {
                prepend,
                append,
                replace,
                remove,
            } => (prepend.iter().chain(append).chain(replace).collect(), remove),
        };
        if rows.len() > MAX_PAGE_ROWS || removed.len() > MAX_PAGE_ROWS {
            return Err(format!("more than {MAX_PAGE_ROWS} items in one answer"));
        }
        for row in &rows {
            row.validate(Source::Items)?;
        }
        for id in removed {
            if id.is_empty() || id.len() > MAX_ID_BYTES || id.chars().any(char::is_control) {
                return Err("an id in `remove` isn't one".into());
            }
        }
        let bytes: usize = rows.iter().map(|row| row.text_bytes()).sum::<usize>()
            + removed.iter().map(String::len).sum::<usize>();
        if bytes > MAX_PAGE_BYTES {
            return Err("more text than one answer may carry".into());
        }
        if self
            .cursor
            .as_ref()
            .is_some_and(|cursor| cursor.len() > MAX_CURSOR_BYTES)
        {
            return Err("`cursor` is too long".into());
        }
        if self.ttl.is_some_and(|ttl| ttl == 0 || ttl > MAX_TTL_SECONDS) {
            return Err("`ttl` is out of range".into());
        }
        Ok(())
    }
}

/// A path inside the package, to an image format the launcher draws.
pub(crate) fn check_icon(icon: &str) -> Result<(), String> {
    let invalid = || {
        Err(format!(
            "`icon` must be a path inside the package to a {} file",
            ICON_EXTENSIONS.join(", ")
        ))
    };
    if icon.is_empty()
        || icon.len() > MAX_ICON_BYTES
        || icon.starts_with('/')
        || icon.contains('\\')
        || icon.chars().any(char::is_control)
        || icon
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return invalid();
    }
    let extension = icon
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase());
    if !extension.is_some_and(|extension| ICON_EXTENSIONS.contains(&extension.as_str())) {
        return invalid();
    }
    Ok(())
}

/// A handle shaped as storage and the clipboard watch hand them out, and for a copied file, an
/// absolute path. Whether the package may draw it is decided when Dupin draws it.
pub(crate) fn check_image(image: &RowImage) -> Result<(), String> {
    let handle = |handle: &str| {
        if handle.is_empty()
            || handle.len() > MAX_IMAGE_HANDLE_BYTES
            || !handle.bytes().all(|byte| byte.is_ascii_graphic())
        {
            Err(format!(
                "must be a handle of 1 to {MAX_IMAGE_HANDLE_BYTES} printable ASCII characters"
            ))
        } else {
            Ok(())
        }
    };
    match image {
        RowImage::Blob(blob) => handle(blob),
        RowImage::File(file) => {
            check_file(&file.path).map_err(|_| {
                format!("must name its file by an absolute path of at most {MAX_FILE_BYTES} bytes")
            })?;
            handle(&file.handle)
        }
    }
}

/// An absolute path to show the Finder icon of. The row only names it; nothing reads it.
pub(crate) fn check_file(file: &str) -> Result<(), String> {
    if !file.starts_with('/') || file.len() > MAX_FILE_BYTES || file.chars().any(char::is_control) {
        return Err(format!(
            "`file` must be an absolute path of at most {MAX_FILE_BYTES} bytes"
        ));
    }
    Ok(())
}

/// What a row may open: a web URL; a `dict://` URL, which only shows a word in Dictionary; or,
/// outside a catalogue, which reaches no host service, the `link` the calendar service gave an
/// event, which only shows that event in Calendar.
pub(crate) fn is_openable(url: &str, source: Source) -> bool {
    if url.len() > MAX_URL_BYTES {
        return false;
    }
    is_web_url(url) || is_dictionary_url(url) || (source == Source::Search && is_calendar_link(url))
}

/// Whether `url` only shows a word in Dictionary, which any row may open without a host
/// service having given it.
pub fn is_dictionary_url(url: &str) -> bool {
    url.len() <= MAX_URL_BYTES
        && url.strip_prefix("dict://").is_some_and(|word| !word.is_empty())
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
}

fn is_calendar_link(url: &str) -> bool {
    url.strip_prefix(CALENDAR_LINK)
        .and_then(|rest| rest.strip_suffix(CALENDAR_LINK_QUERY))
        .is_some_and(|event| {
            !event.is_empty()
                && event
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"%-._~/".contains(&byte))
        })
}

const CALENDAR_LINK: &str = "ical://ekevent/";
const CALENDAR_LINK_QUERY: &str = "?method=show&options=more";

/// The name of an export in the package's `actions` object.
pub(crate) fn check_handler(handler: &str) -> Result<(), String> {
    let valid = !handler.is_empty()
        && handler.len() <= MAX_HANDLER_BYTES
        && !handler.starts_with(|c: char| c.is_ascii_digit())
        && handler
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if valid {
        Ok(())
    } else {
        Err(format!(
            "`run` must name an action handler: up to {MAX_HANDLER_BYTES} letters, digits, `_` or `$`"
        ))
    }
}

pub(crate) fn check_data(data: &serde_json::Value) -> Result<(), String> {
    fn depth(value: &serde_json::Value) -> usize {
        match value {
            serde_json::Value::Array(values) => 1 + values.iter().map(depth).max().unwrap_or(0),
            serde_json::Value::Object(values) => 1 + values.values().map(depth).max().unwrap_or(0),
            _ => 0,
        }
    }
    if data.to_string().len() > MAX_DATA_BYTES {
        return Err(format!(
            "`data` is more than {} KiB of JSON",
            MAX_DATA_BYTES / 1024
        ));
    }
    if depth(data) > MAX_DATA_DEPTH {
        return Err(format!("`data` nests more than {MAX_DATA_DEPTH} levels deep"));
    }
    Ok(())
}
