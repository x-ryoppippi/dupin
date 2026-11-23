//! Clipboard history: what the platform layer captured from the system clipboard, kept in
//! memory, deduplicated, bounded and searchable. Capturing and writing the clipboard belong to
//! the platform layer; this module only decides what is kept and how it is found.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::config;
use crate::item::tilde;
use crate::{Action, Item, ItemAction, Payload, Provider, ProviderError, Request, View};

const TITLE_CHARS: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    Text(String),
    Files(Vec<PathBuf>),
}

impl Content {
    fn bytes(&self) -> usize {
        match self {
            Content::Text(text) => text.len(),
            Content::Files(paths) => paths.iter().map(|path| path.as_os_str().len()).sum(),
        }
    }

    fn is_blank(&self) -> bool {
        match self {
            Content::Text(text) => text.trim().is_empty(),
            Content::Files(paths) => paths.is_empty(),
        }
    }

    /// One line to show and to highlight matches in.
    fn title(&self) -> String {
        match self {
            Content::Text(text) => {
                let mut title = String::new();
                for (index, word) in text.split_whitespace().enumerate() {
                    if index > 0 {
                        title.push(' ');
                    }
                    title.push_str(word);
                    if title.chars().count() >= TITLE_CHARS {
                        break;
                    }
                }
                match title.char_indices().nth(TITLE_CHARS) {
                    Some((end, _)) => format!("{}…", &title[..end]),
                    None => title,
                }
            }
            Content::Files(paths) => paths
                .iter()
                .map(|path| {
                    path.file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy()
                        .into_owned()
                })
                .collect::<Vec<_>>()
                .join(", "),
        }
    }

    fn search_fields(&self) -> Vec<String> {
        match self {
            Content::Text(text) => {
                let lower = text.to_lowercase();
                if lower == *text { Vec::new() } else { vec![lower] }
            }
            Content::Files(paths) => paths
                .iter()
                .map(|path| path.to_string_lossy().to_lowercase())
                .collect(),
        }
    }
}

/// Whether the system lets Dupin read the clipboard in the background.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Access {
    #[default]
    Allowed,
    /// Reading would raise the system's paste prompt, which Dupin has never raised. Only a
    /// deliberate request may raise it, never a background read.
    Unasked,
    /// The system asks or refuses on every read; the user changes that in System Settings.
    Restricted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    id: u64,
    content: Content,
    /// Unix seconds.
    copied_at: u64,
    search_fields: Vec<String>,
}

impl Entry {
    fn matches(&self, words: &[String]) -> bool {
        if let Content::Text(text) = &self.content
            && self.search_fields.is_empty()
        {
            return words.iter().all(|word| text.contains(word));
        }
        words
            .iter()
            .all(|word| self.search_fields.iter().any(|field| field.contains(word)))
    }
}

/// What the history keeps, from `[clipboard]` in config.toml.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    max_entries: usize,
    pub max_entry_bytes: usize,
    ignored_apps: Vec<String>,
}

impl From<&config::Clipboard> for Policy {
    fn from(config: &config::Clipboard) -> Self {
        Self {
            max_entries: config.max_entries,
            max_entry_bytes: config.max_entry_bytes,
            ignored_apps: config.ignored_apps.clone(),
        }
    }
}

impl Default for Policy {
    fn default() -> Self {
        (&config::Clipboard::default()).into()
    }
}

#[derive(Debug, Default)]
pub struct History {
    /// Newest first.
    entries: VecDeque<Arc<Entry>>,
    next_id: u64,
    policy: Policy,
    access: Access,
}

impl History {
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            ..Self::default()
        }
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// Apply new limits, dropping what no longer fits.
    pub fn set_policy(&mut self, policy: Policy) {
        self.entries
            .retain(|entry| entry.content.bytes() <= policy.max_entry_bytes);
        self.entries.truncate(policy.max_entries);
        self.policy = policy;
    }

    /// Whether the source app is allowed to contribute to history.
    pub fn accepts_source(&self, source: Option<&str>) -> bool {
        !source.is_some_and(|source| self.policy.ignored_apps.iter().any(|app| app == source))
    }

    /// Record a copy as the newest entry. An equal entry moves to the front instead of
    /// repeating. Blank and oversized copies are not recorded.
    pub fn push(&mut self, content: Content, now: u64) -> bool {
        if content.is_blank() || content.bytes() > self.policy.max_entry_bytes {
            return false;
        }
        self.entries.retain(|entry| entry.content != content);
        self.next_id += 1;
        self.entries.push_front(Arc::new(Entry {
            id: self.next_id,
            search_fields: content.search_fields(),
            content,
            copied_at: now,
        }));
        self.entries.truncate(self.policy.max_entries);
        true
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.id != id);
        self.entries.len() != before
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    #[cfg(test)]
    fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().map(Arc::as_ref)
    }

    pub fn set_access(&mut self, access: Access) {
        self.access = access;
    }

    /// Entries containing every word of the query, ignoring case, newest first, with the
    /// title characters that matched. An empty query lists the newest entries.
    #[cfg(test)]
    fn search(&self, query: &str, limit: usize) -> Vec<(&Entry, String, Vec<usize>)> {
        search_entries(self.entries(), query, limit)
    }
}

fn search_entries<'a>(
    entries: impl Iterator<Item = &'a Entry>,
    query: &str,
    limit: usize,
) -> Vec<(&'a Entry, String, Vec<usize>)> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    entries
        .filter(|entry| entry.matches(&words))
        .take(limit)
        .map(|entry| {
            let title = entry.content.title();
            let matched = highlights(&title, &words);
            (entry, title, matched)
        })
        .collect()
}

/// Character indices of each word's first occurrence in `title`, ignoring case.
fn highlights(title: &str, words: &[String]) -> Vec<usize> {
    let lower: Vec<char> = title
        .chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect();
    let mut matched: Vec<usize> = words
        .iter()
        .filter_map(|word| {
            let word: Vec<char> = word.chars().collect();
            (!word.is_empty() && word.len() <= lower.len())
                .then(|| {
                    lower
                        .windows(word.len())
                        .position(|window| window == word.as_slice())
                })
                .flatten()
                .map(|start| start..start + word.len())
        })
        .flatten()
        .collect();
    matched.sort_unstable();
    matched.dedup();
    matched
}

fn ago(now: u64, then: u64) -> String {
    let seconds = now.saturating_sub(then);
    match seconds {
        0..60 => "Just now".into(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..86_400 => format!("{} h ago", seconds / 3600),
        _ => format!("{} d ago", seconds / 86_400),
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn item(entry: &Entry, title: String, matched: Vec<usize>, now: u64) -> Item {
    let when = ago(now, entry.copied_at);
    let (subtitle, icon, payload, copy) = match &entry.content {
        Content::Text(text) => {
            let lines = text.lines().count();
            let size = if lines > 1 {
                plural(lines, "line", "lines")
            } else {
                plural(text.chars().count(), "character", "characters")
            };
            (
                format!("{when} · {size}"),
                None,
                Payload::text(text.clone()),
                Action::Copy(text.clone()),
            )
        }
        Content::Files(paths) => {
            let place = match paths.as_slice() {
                [path] => path
                    .parent()
                    .map(|parent| tilde(&parent.to_string_lossy()))
                    .unwrap_or_default(),
                paths => plural(paths.len(), "file", "files"),
            };
            (
                format!("{when} · {place}"),
                paths.first().cloned(),
                Payload {
                    text: None,
                    paths: paths.clone(),
                },
                Action::CopyFiles(paths.clone()),
            )
        }
    };
    Item {
        id: format!("clipboard-{}", entry.id),
        payload,
        view: View::Text { title, subtitle },
        record_usage: false,
        icon,
        matched,
        completion: None,
        score: None,
        primary: Some("copy".into()),
        alternate: None,
        actions: vec![
            ItemAction::new("copy", "Copy", copy),
            ItemAction::new(
                "delete",
                "Delete from History",
                Action::DeleteClipboardEntry(entry.id),
            )
            .consumes(),
            ItemAction::new("clear", "Clear History", Action::ClearClipboardHistory)
                .confirm("Clear the whole clipboard history?", "Clear History")
                .consumes(),
        ],
    }
}

pub struct ClipboardProvider {
    history: Arc<Mutex<History>>,
    access_item: fn(Access) -> Option<Item>,
    /// Unix seconds; a parameter so tests can fix the time.
    clock: fn() -> u64,
}

impl ClipboardProvider {
    pub fn new(history: Arc<Mutex<History>>, access_item: fn(Access) -> Option<Item>) -> Self {
        Self {
            history,
            access_item,
            clock: crate::session::unix_now,
        }
    }
}

impl Provider for ClipboardProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        let now = (self.clock)();
        let (access, entries) = {
            let history = self.history.lock().map_err(|_| ProviderError)?;
            (history.access, history.entries.clone())
        };
        let mut items: Vec<Item> = (self.access_item)(access).into_iter().collect();
        let candidates = entries
            .iter()
            .take_while(|_| !request.is_cancelled())
            .map(Arc::as_ref);
        let limit = request.limit.saturating_sub(items.len());
        items.extend(
            search_entries(candidates, &request.query, limit)
                .into_iter()
                .map(|(entry, title, matched)| item(entry, title, matched, now)),
        );
        emit(items);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(max_entries: usize, max_entry_bytes: usize) -> Policy {
        Policy {
            max_entries,
            max_entry_bytes,
            ignored_apps: vec!["com.example.vault".into()],
        }
    }

    fn text(value: &str) -> Content {
        Content::Text(value.into())
    }

    fn texts(history: &History) -> Vec<String> {
        history
            .entries()
            .map(|entry| match &entry.content {
                Content::Text(text) => text.clone(),
                Content::Files(paths) => format!("{paths:?}"),
            })
            .collect()
    }

    #[test]
    fn newest_first_with_duplicates_moved_to_the_front() {
        let mut history = History::new(policy(10, 1024));
        assert!(history.push(text("alpha"), 1));
        assert!(history.push(text("beta"), 2));
        assert!(history.push(text("alpha"), 3));
        assert_eq!(texts(&history), ["alpha", "beta"]);
        let first = history.entries().next().unwrap();
        assert_eq!(first.copied_at, 3);
        let ids: Vec<_> = history.entries().map(|entry| entry.id).collect();
        assert_ne!(ids[0], ids[1]);
    }

    #[test]
    fn blank_and_oversized_copies_are_skipped_not_truncated() {
        let mut history = History::new(policy(10, 8));
        assert!(!history.push(text(" \n\t"), 1));
        assert!(!history.push(text("123456789"), 1));
        assert!(!history.push(Content::Files(Vec::new()), 1));
        assert!(history.push(text("12345678"), 1));
        assert_eq!(texts(&history), ["12345678"]);
    }

    #[test]
    fn count_limit_drops_the_oldest_and_new_limits_apply_at_once() {
        let mut history = History::new(policy(3, 1024));
        for (now, value) in ["one", "two", "three", "four"].into_iter().enumerate() {
            history.push(text(value), now as u64);
        }
        assert_eq!(texts(&history), ["four", "three", "two"]);
        history.set_policy(policy(2, 4));
        assert_eq!(texts(&history), ["four", "two"]);
        history.set_policy(policy(1, 1024));
        assert_eq!(texts(&history), ["four"]);
    }

    #[test]
    fn ignored_sources_are_refused() {
        let history = History::new(policy(10, 1024));
        assert!(history.accepts_source(Some("com.example.editor")));
        assert!(history.accepts_source(None));
        assert!(!history.accepts_source(Some("com.example.vault")));
        assert!(history.accepts_source(Some("com.example.vault.helper")));
    }

    #[test]
    fn remove_and_clear() {
        let mut history = History::new(policy(10, 1024));
        history.push(text("keep"), 1);
        history.push(text("drop"), 2);
        let drop = history.entries().next().unwrap().id;
        assert!(history.remove(drop));
        assert!(!history.remove(drop));
        assert_eq!(texts(&history), ["keep"]);
        history.clear();
        assert!(history.entries.is_empty());
    }

    #[test]
    fn search_requires_every_word_ignoring_case_and_highlights_the_title() {
        let mut history = History::new(policy(10, 1024));
        history.push(text("Invoice number 42\nDue Friday"), 1);
        history.push(text("meeting notes"), 2);
        history.push(Content::Files(vec!["/fixture/Reports/Invoice.pdf".into()]), 3);

        let hits = history.search("INVOICE due", 10);
        assert_eq!(hits.len(), 1);
        let (_, title, matched) = &hits[0];
        assert_eq!(title, "Invoice number 42 Due Friday");
        assert_eq!(matched, &[0, 1, 2, 3, 4, 5, 6, 18, 19, 20]);

        let hits = history.search("invoice", 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].1, "Invoice.pdf");

        let hits = history.search("reports", 10);
        assert_eq!(hits.len(), 1, "file entries match their whole path");
        assert!(hits[0].2.is_empty());

        assert_eq!(history.search("", 2).len(), 2);
        assert!(history.search("absent", 10).is_empty());
    }

    #[test]
    fn search_matches_full_unicode_text_beyond_the_title() {
        let mut history = History::default();
        let original = format!("{} TÜRKİYE Café", "x".repeat(TITLE_CHARS + 50));
        assert!(history.push(text(&original), 1));
        let hits = history.search("CAFÉ türki\u{307}ye", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0.content, text(&original));
        assert!(hits[0].2.is_empty());
    }

    #[test]
    fn titles_collapse_whitespace_and_stay_short() {
        assert_eq!(text("  a\n\n b\tc  ").title(), "a b c");
        let long = "x".repeat(TITLE_CHARS + 50);
        let title = text(&long).title();
        assert_eq!(title.chars().count(), TITLE_CHARS + 1);
        assert!(title.ends_with('…'));
        let files = Content::Files(vec!["/a/one.txt".into(), "/b/two.txt".into()]);
        assert_eq!(files.title(), "one.txt, two.txt");
    }

    #[test]
    fn provider_rows_copy_delete_and_confirm_clearing_without_usage_history() {
        let history = Arc::new(Mutex::new(History::new(policy(10, 1024))));
        history.lock().unwrap().push(text("first line\nsecond line"), 100);
        history
            .lock()
            .unwrap()
            .push(Content::Files(vec!["/fixture/a.txt".into()]), 160);
        let provider = ClipboardProvider {
            history: history.clone(),
            access_item: |access| {
                (access != Access::Allowed).then(|| {
                    Item::path(
                        "/fixture/clipboard-access".into(),
                        "Clipboard access".into(),
                        vec![],
                    )
                })
            },
            clock: || 220,
        };
        let mut rows = Vec::new();
        provider
            .search(&Request::detached("", 10), &mut |batch| rows.extend(batch))
            .unwrap();
        assert_eq!(rows.len(), 2);

        let file = &rows[0];
        assert_eq!(file.display_title(), "a.txt");
        assert_eq!(
            file.view,
            View::Text {
                title: "a.txt".into(),
                subtitle: "1 min ago · /fixture".into()
            }
        );
        assert_eq!(
            file.action("copy"),
            Some(&Action::CopyFiles(vec!["/fixture/a.txt".into()]))
        );

        let note = &rows[1];
        assert!(!note.record_usage);
        assert_eq!(note.primary.as_deref(), Some("copy"));
        assert_eq!(
            note.view,
            View::Text {
                title: "first line second line".into(),
                subtitle: "2 min ago · 2 lines".into()
            }
        );
        assert_eq!(
            note.action("copy"),
            Some(&Action::Copy("first line\nsecond line".into()))
        );
        assert!(note.item_action("delete").unwrap().confirmation().is_none());
        assert!(note.item_action("clear").unwrap().confirmation().is_some());

        for access in [Access::Unasked, Access::Restricted] {
            history.lock().unwrap().set_access(access);
            let mut rows = Vec::new();
            provider
                .search(&Request::detached("line", 1), &mut |batch| rows.extend(batch))
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].id, "/fixture/clipboard-access");
        }
    }

    #[test]
    fn ages_read_naturally() {
        assert_eq!(ago(100, 100), "Just now");
        assert_eq!(ago(100, 200), "Just now");
        assert_eq!(ago(3599, 0), "59 min ago");
        assert_eq!(ago(7200, 0), "2 h ago");
        assert_eq!(ago(172_800, 0), "2 d ago");
    }
}
