//! Text snippets from config.toml: which ones are usable, how they match a query, and how
//! their placeholders are filled in when one is used.

use std::sync::{Arc, RwLock};

use crate::config::Config;
use crate::{Action, Item, ItemAction, Payload, Provider, ProviderError, Request, View, fuzzy};

const MAX_KEYWORD_BYTES: usize = 32;
const MAX_NAME_CHARS: usize = 80;
const MAX_TEXT_BYTES: usize = 64 * 1024;
const PREVIEW_CHARS: usize = 80;

/// The wall-clock time where the user is, as placeholders show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Placeholder {
    Date,
    Time,
    DateTime,
    Clipboard,
}

const CLIPBOARD_PLACEHOLDER: &str = "{clipboard}";
const PLACEHOLDERS: [(&str, Placeholder); 4] = [
    ("{date}", Placeholder::Date),
    ("{time}", Placeholder::Time),
    ("{datetime}", Placeholder::DateTime),
    (CLIPBOARD_PLACEHOLDER, Placeholder::Clipboard),
];

/// The next placeholder at or after `from`: its byte range and kind. Braces that don't spell
/// one are ordinary text.
fn next_placeholder(text: &str, from: usize) -> Option<(usize, usize, Placeholder)> {
    text[from..].match_indices('{').find_map(|(offset, _)| {
        let start = from + offset;
        PLACEHOLDERS
            .iter()
            .find(|(token, _)| text[start..].starts_with(token))
            .map(|(token, kind)| (start, start + token.len(), *kind))
    })
}

/// Whether expanding the snippet requires clipboard text.
pub fn needs_clipboard(text: &str) -> bool {
    text.contains(CLIPBOARD_PLACEHOLDER)
}

/// Fill the placeholders using the activation time and any clipboard text already read.
pub fn expand(text: &str, now: LocalTime, clipboard: &str) -> String {
    let date = format!("{:04}-{:02}-{:02}", now.year, now.month, now.day);
    let time = format!("{:02}:{:02}", now.hour, now.minute);
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    while let Some((start, end, kind)) = next_placeholder(text, from) {
        out.push_str(&text[from..start]);
        match kind {
            Placeholder::Date => out.push_str(&date),
            Placeholder::Time => out.push_str(&time),
            Placeholder::DateTime => {
                out.push_str(&date);
                out.push(' ');
                out.push_str(&time);
            }
            Placeholder::Clipboard => out.push_str(clipboard),
        }
        from = end;
    }
    out.push_str(&text[from..]);
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    keyword: String,
    title: String,
    text: String,
}

/// The usable snippets from config.toml, and whether Enter pastes them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SnippetCatalog {
    entries: Vec<Entry>,
    paste: bool,
}

impl SnippetCatalog {
    /// The valid snippets, and a notice when some were left out.
    pub fn compile(config: &Config) -> (Self, Option<String>) {
        let mut entries = Vec::new();
        let mut skipped = false;
        for (keyword, snippet) in &config.snippets {
            let name = snippet.name.as_deref();
            if !valid_keyword(keyword) || !name.is_none_or(valid_name) || !valid_text(&snippet.text) {
                skipped = true;
                continue;
            }
            entries.push(Entry {
                keyword: keyword.clone(),
                title: name.unwrap_or(keyword).to_owned(),
                text: snippet.text.clone(),
            });
        }
        let notice = skipped.then(|| {
            "Some snippets were ignored because a keyword, name or text is invalid. Review snippets in config.toml."
                .to_owned()
        });
        (
            Self {
                entries,
                paste: config.snippet_paste,
            },
            notice,
        )
    }

    /// The snippets matching `query`, best first; an empty query lists them all in keyword
    /// order.
    fn search(&self, query: &str, limit: usize, is_cancelled: impl Fn() -> bool) -> Vec<Item> {
        let query = query.trim();
        let mut items = Vec::new();
        for entry in &self.entries {
            if is_cancelled() {
                return Vec::new();
            }
            if query.is_empty() {
                items.push(self.item(entry, 0, Vec::new()));
                continue;
            }
            let title = fuzzy::score(query, &entry.title);
            let keyword = (entry.keyword != entry.title)
                .then(|| fuzzy::score(query, &entry.keyword))
                .flatten();
            let found = match (title, keyword) {
                (Some(title), Some(keyword)) if keyword.score > title.score => {
                    Some((keyword.score, Vec::new()))
                }
                (Some(title), _) => Some((title.score, title.positions)),
                (None, Some(keyword)) => Some((keyword.score, Vec::new())),
                (None, None) => None,
            };
            if let Some((score, matched)) = found {
                items.push(self.item(entry, score, matched));
            }
        }
        items.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
        items.truncate(limit);
        items
    }

    fn item(&self, entry: &Entry, score: i32, matched: Vec<usize>) -> Item {
        let copy = ItemAction::new(
            "copy",
            "Copy to Clipboard",
            Action::Snippet {
                text: entry.text.clone(),
                paste: false,
            },
        );
        let (primary, alternate, actions) = if self.paste {
            let paste = ItemAction::new(
                "paste",
                "Paste into Front App",
                Action::Snippet {
                    text: entry.text.clone(),
                    paste: true,
                },
            );
            ("paste", Some("copy".into()), vec![paste, copy])
        } else {
            ("copy", None, vec![copy])
        };
        Item {
            id: format!("snippet:{}", entry.keyword),
            payload: if next_placeholder(&entry.text, 0).is_none() {
                Payload::text(entry.text.clone())
            } else {
                Payload::default()
            },
            view: View::Text {
                title: entry.title.clone(),
                subtitle: format!("{}  ·  {}", entry.keyword, preview(&entry.text)),
            },
            record_usage: true,
            icon: None,
            matched,
            completion: None,
            score: Some(score),
            primary: Some(primary.into()),
            alternate,
            actions,
        }
    }
}

fn valid_keyword(keyword: &str) -> bool {
    !keyword.is_empty()
        && keyword.len() <= MAX_KEYWORD_BYTES
        && !keyword.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.chars().count() <= MAX_NAME_CHARS && !name.chars().any(char::is_control)
}

fn valid_text(text: &str) -> bool {
    !text.is_empty() && text.len() <= MAX_TEXT_BYTES
}

/// The text on one line, whitespace runs collapsed, cut to fit a subtitle.
fn preview(text: &str) -> String {
    let mut words = text.split_whitespace();
    let mut out = words.next().unwrap_or_default().to_owned();
    for word in words {
        out.push(' ');
        out.push_str(word);
    }
    if out.chars().count() > PREVIEW_CHARS {
        out = out.chars().take(PREVIEW_CHARS - 1).collect();
        out.push('…');
    }
    out
}

/// Snippet search over a catalogue the host replaces when config.toml changes.
pub struct SnippetProvider {
    catalog: Arc<RwLock<SnippetCatalog>>,
}

impl SnippetProvider {
    pub fn new(catalog: Arc<RwLock<SnippetCatalog>>) -> Self {
        Self { catalog }
    }
}

impl Provider for SnippetProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        let items = {
            let catalog = self.catalog.read().map_err(|_| ProviderError)?;
            catalog.search(&request.query, request.limit, || request.is_cancelled())
        };
        if !items.is_empty() && !request.is_cancelled() {
            emit(items);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::config::Snippet;

    const NOW: LocalTime = LocalTime {
        year: 2026,
        month: 1,
        day: 31,
        hour: 9,
        minute: 5,
    };

    fn snippet(name: Option<&str>, text: &str) -> Snippet {
        Snippet {
            name: name.map(str::to_owned),
            text: text.into(),
        }
    }

    fn config(paste: bool) -> Config {
        Config {
            snippets: BTreeMap::from([
                (
                    "sig".into(),
                    snippet(Some("Email signature"), "Best wishes,\n  Dupin"),
                ),
                ("addr".into(), snippet(Some("Home address"), "1 Baker Street")),
                ("today".into(), snippet(None, "{date}")),
            ]),
            snippet_paste: paste,
            ..Config::default()
        }
    }

    fn search(catalog: &SnippetCatalog, query: &str) -> Vec<Item> {
        catalog.search(query, 10, || false)
    }

    fn titles(items: &[Item]) -> Vec<&str> {
        items.iter().map(Item::display_title).collect()
    }

    #[test]
    fn fills_in_date_and_time_placeholders() {
        assert_eq!(expand("{date}", NOW, ""), "2026-01-31");
        assert_eq!(expand("at {time}", NOW, ""), "at 09:05");
        assert_eq!(expand("{datetime}.", NOW, ""), "2026-01-31 09:05.");
        let year = LocalTime { year: 987, ..NOW };
        assert_eq!(expand("{date}", year, ""), "0987-01-31");
    }

    #[test]
    fn clipboard_placeholders_share_the_same_text() {
        let template = "{clipboard} and {{clipboard}}";
        assert!(needs_clipboard(template));
        assert!(!needs_clipboard("fn main() {} {date} {Clipboard} {clipboard"));
        assert_eq!(expand(template, NOW, "pasted"), "pasted and {pasted}");
        assert_eq!(expand("[{clipboard}]", NOW, ""), "[]");
    }

    #[test]
    fn other_braces_stay_as_written() {
        assert_eq!(expand("fn main() {}", NOW, ""), "fn main() {}");
        assert_eq!(expand("{{date}}", NOW, ""), "{2026-01-31}");
        assert_eq!(expand("{Date} {date", NOW, ""), "{Date} {date");
        assert_eq!(expand("日本{time}語", NOW, ""), "日本09:05語");
        assert_eq!(expand("", NOW, ""), "");
    }

    #[test]
    fn empty_query_lists_every_snippet_by_keyword() {
        let (catalog, notice) = SnippetCatalog::compile(&config(false));
        assert!(notice.is_none());
        let items = search(&catalog, "  ");
        assert_eq!(titles(&items), ["Home address", "Email signature", "today"]);
        assert!(items.iter().all(|item| item.score == Some(0)));
        assert_eq!(catalog.search("", 2, || false).len(), 2);
    }

    #[test]
    fn matches_names_and_keywords() {
        let (catalog, _) = SnippetCatalog::compile(&config(false));
        let items = search(&catalog, "email");
        assert_eq!(titles(&items), ["Email signature"]);
        assert_eq!(items[0].matched, [0, 1, 2, 3, 4]);

        let items = search(&catalog, "sig");
        assert_eq!(titles(&items)[0], "Email signature");
        assert!(search(&catalog, "xyz").is_empty());
        assert_eq!(titles(&search(&catalog, "today")), ["today"]);
    }

    #[test]
    fn rows_preview_the_text_and_copy_unless_paste_is_enabled() {
        let (catalog, _) = SnippetCatalog::compile(&config(false));
        let item = search(&catalog, "sig").remove(0);
        assert_eq!(
            item.view,
            View::Text {
                title: "Email signature".into(),
                subtitle: "sig  ·  Best wishes, Dupin".into(),
            }
        );
        assert_eq!(item.id, "snippet:sig");
        assert!(item.record_usage);
        assert_eq!(item.primary.as_deref(), Some("copy"));
        assert_eq!(item.alternate, None);
        assert_eq!(
            item.action("copy"),
            Some(&Action::Snippet {
                text: "Best wishes,\n  Dupin".into(),
                paste: false,
            })
        );
        assert!(item.action("paste").is_none());

        let (catalog, _) = SnippetCatalog::compile(&config(true));
        let item = search(&catalog, "sig").remove(0);
        assert_eq!(item.primary.as_deref(), Some("paste"));
        assert_eq!(item.alternate.as_deref(), Some("copy"));
        assert_eq!(
            item.action("paste"),
            Some(&Action::Snippet {
                text: "Best wishes,\n  Dupin".into(),
                paste: true,
            })
        );
    }

    #[test]
    fn long_previews_are_cut() {
        let text = "word ".repeat(40);
        let shown = preview(&text);
        assert_eq!(shown.chars().count(), PREVIEW_CHARS);
        assert!(shown.ends_with('…'));
    }

    #[test]
    fn invalid_snippets_are_skipped_with_a_notice() {
        let mut config = config(false);
        config.snippets.insert("two words".into(), snippet(None, "x"));
        config.snippets.insert("empty".into(), snippet(None, ""));
        config.snippets.insert("blank".into(), snippet(Some("  "), "x"));
        config
            .snippets
            .insert("huge".into(), snippet(None, &"x".repeat(MAX_TEXT_BYTES + 1)));
        let (catalog, notice) = SnippetCatalog::compile(&config);
        assert!(notice.unwrap().contains("snippets"));
        assert_eq!(search(&catalog, "").len(), 3);
    }

    #[test]
    fn provider_observes_a_replaced_catalogue() {
        let (catalog, _) = SnippetCatalog::compile(&config(false));
        let catalog = Arc::new(RwLock::new(catalog));
        let provider = SnippetProvider::new(catalog.clone());
        let collect = |query: &str| {
            let mut items = Vec::new();
            provider
                .search(&Request::detached(query, 10), &mut |batch| items.extend(batch))
                .unwrap();
            items
        };
        assert_eq!(collect("").len(), 3);
        *catalog.write().unwrap() = SnippetCatalog::compile(&Config::default()).0;
        assert!(collect("").is_empty());
    }
}
