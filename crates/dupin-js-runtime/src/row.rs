//! The rows an extension answers with, after the reader has checked them, and the same checks
//! again for rows that crossed from the extension host, which Dupin can't trust.

use dupin_plugin_protocol::{MAX_URL_BYTES, is_web_url};
use serde::{Deserialize, Serialize};

use crate::web_search::WebSearch;

pub(crate) const MAX_TITLE_BYTES: usize = 512;
pub(crate) const MAX_SUBTITLE_BYTES: usize = 2048;
pub(crate) const MAX_ID_BYTES: usize = 128;
pub(crate) const MAX_TEXT_BYTES: usize = 16 * 1024;
pub(crate) const MAX_KEYWORDS: usize = 16;
pub(crate) const MAX_KEYWORD_BYTES: usize = 64;
pub(crate) const MAX_SECTION_BYTES: usize = 128;
pub(crate) const MAX_ICON_BYTES: usize = 256;
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
    /// A package-relative image path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<RowDetail>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<RowAction>,
    /// Makes the catalogue row a web search, which Dupin fills and opens itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_search: Option<WebSearch>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RowDetail {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub markdown: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metadata: Vec<Metadata>,
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

/// Which rows a check applies to: `keywords` only mean something to a catalogue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Search,
    Items,
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
        if source == Source::Search && !self.keywords.is_empty() {
            return Err("`keywords` belong to items()".into());
        }
        if let Some(web_search) = &self.web_search {
            if source == Source::Search {
                return Err("`webSearch` belongs to items()".into());
            }
            web_search.validate()?;
            if !self.actions.is_empty() {
                return Err("a web search opens its own URL, so it has no `url`, `text` or `actions`".into());
            }
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
        if let Some(section) = &self.section {
            line("section", section, MAX_SECTION_BYTES)?;
        }
        if let Some(detail) = &self.detail {
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
        for action in &self.actions {
            line("actions.title", &action.title, MAX_ACTION_TITLE_BYTES)?;
            match &action.effect {
                RowEffect::Copy { text } if text.is_empty() || text.len() > MAX_TEXT_BYTES => {
                    return Err("a copy action's text must be 1 byte to 16 KiB".into());
                }
                RowEffect::Paste { text } if text.is_empty() || text.len() > MAX_TEXT_BYTES => {
                    return Err("a paste action's text must be 1 byte to 16 KiB".into());
                }
                RowEffect::OpenUrl { url } if source == Source::Items && !is_web_url(url) => {
                    return Err("a catalogue's open action needs an http or https URL".into());
                }
                RowEffect::OpenUrl { url } if !is_openable(url) => {
                    return Err(
                        "an open action needs an http or https URL, or a calendar event's link".into(),
                    );
                }
                RowEffect::Run { handler, data } => {
                    check_handler(handler)?;
                    if let Some(data) = data {
                        check_data(data)?;
                    }
                }
                RowEffect::Copy { .. } | RowEffect::Paste { .. } | RowEffect::OpenUrl { .. } => {}
            }
        }
        Ok(())
    }

    /// The text the row carries, counted toward an answer's limit.
    pub fn text_bytes(&self) -> usize {
        let detail = self.detail.as_ref().map_or(0, |detail| {
            detail.markdown.len()
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
                    + match &action.effect {
                        RowEffect::Copy { text } | RowEffect::Paste { text } => text.len(),
                        RowEffect::OpenUrl { url } => url.len(),
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
            + self.icon.as_ref().map_or(0, String::len)
            + self.section.as_ref().map_or(0, String::len)
            + self.web_search.as_ref().map_or(0, WebSearch::text_bytes)
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
                append,
                replace,
                remove,
            } => (append.iter().chain(replace).collect(), remove),
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

/// What an open action may open: a web URL, or the `link` the calendar service gives an event,
/// which only shows that event in Calendar.
pub(crate) fn is_openable(url: &str) -> bool {
    if url.len() > MAX_URL_BYTES {
        return false;
    }
    is_web_url(url)
        || url
            .strip_prefix(CALENDAR_LINK)
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
