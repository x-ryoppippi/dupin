//! Clipboard history: what the platform layer captured from the system clipboard, deduplicated,
//! bounded, expired and searchable, and written through to a [`Store`]. Capturing and writing the
//! clipboard belong to the platform layer; this module only decides what is kept and how it is
//! found.

mod store;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use zeroize::Zeroize;

use crate::config;
use crate::item::tilde;
use crate::{Action, Item, ItemAction, Payload, Provider, ProviderError, Request, View};

pub use store::{MemoryStore, Store, StoreError};

const TITLE_CHARS: usize = 200;
/// Images are most of a history's bytes. Newest first, an image that would take their total past
/// this is dropped, so the entry limit alone can't hold hundreds of megabytes.
const IMAGE_MEMORY: usize = 128 * 1024 * 1024;

/// What one copy held, in the one representation the history keeps of it.
#[derive(Debug, PartialEq, Eq)]
pub enum Content {
    Text(String),
    Image(Image),
    Files(Vec<FileRef>),
}

/// A copied image, converted to PNG.
#[derive(Debug, PartialEq, Eq)]
pub struct Image {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// A copied file, kept by reference: the history never copies the file itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRef {
    pub path: PathBuf,
    /// Bytes, when the file could be measured as it was copied.
    pub size: Option<u64>,
    /// The file's uniform type identifier, such as `public.mpeg-4`.
    pub kind: Option<String>,
}

impl FileRef {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            size: None,
            kind: None,
        }
    }

    /// The paths of `files` that are still there: the only ones a row offers or puts back.
    pub fn present(files: &[FileRef], exists: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
        files
            .iter()
            .filter(|file| exists(&file.path))
            .map(|file| file.path.clone())
            .collect()
    }

    fn name(&self) -> String {
        self.path
            .file_name()
            .unwrap_or(self.path.as_os_str())
            .to_string_lossy()
            .into_owned()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Text,
    Image,
    Files,
}

impl Drop for Content {
    fn drop(&mut self) {
        match self {
            Content::Text(text) => text.zeroize(),
            Content::Image(image) => image.png.zeroize(),
            Content::Files(_) => {}
        }
    }
}

impl Content {
    fn kind(&self) -> Kind {
        match self {
            Content::Text(_) => Kind::Text,
            Content::Image(_) => Kind::Image,
            Content::Files(_) => Kind::Files,
        }
    }

    fn bytes(&self) -> usize {
        match self {
            Content::Text(text) => text.len(),
            Content::Image(image) => image.png.len(),
            Content::Files(files) => files.iter().map(|file| file.path.as_os_str().len()).sum(),
        }
    }

    fn is_blank(&self) -> bool {
        match self {
            Content::Text(text) => text.trim().is_empty(),
            Content::Image(image) => image.png.is_empty(),
            Content::Files(files) => files.is_empty(),
        }
    }

    /// Whether copying this again is the same copy: files by path, as their size may change.
    fn same(&self, other: &Content) -> bool {
        match (self, other) {
            (Content::Files(a), Content::Files(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.path == b.path)
            }
            _ => self == other,
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
            Content::Image(image) => format!("Image · {}×{} PNG", image.width, image.height),
            Content::Files(files) => files.iter().map(FileRef::name).collect::<Vec<_>>().join(", "),
        }
    }
}

/// One entry as a store keeps it.
#[derive(Debug, PartialEq, Eq)]
pub struct Record {
    id: u64,
    pub content: Content,
    /// Unix seconds.
    copied_at: u64,
    /// The name of the app the copy came from, when known.
    pub source: Option<String>,
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

#[derive(Debug)]
struct Entry {
    record: Arc<Record>,
    search_fields: Vec<String>,
}

impl Drop for Entry {
    fn drop(&mut self) {
        self.search_fields.iter_mut().for_each(Zeroize::zeroize);
    }
}

impl Entry {
    fn new(record: Arc<Record>) -> Self {
        Self {
            search_fields: search_fields(&record),
            record,
        }
    }

    fn matches(&self, words: &[String]) -> bool {
        if let Content::Text(text) = &self.record.content
            && self.search_fields.is_empty()
        {
            return words.iter().all(|word| text.contains(word));
        }
        words
            .iter()
            .all(|word| self.search_fields.iter().any(|field| field.contains(word)))
    }
}

fn search_fields(record: &Record) -> Vec<String> {
    match &record.content {
        Content::Text(text) => {
            let lower = text.to_lowercase();
            if lower == *text { Vec::new() } else { vec![lower] }
        }
        Content::Image(image) => {
            let mut fields = vec![
                "image png".to_owned(),
                format!(
                    "{}×{} {}x{}",
                    image.width, image.height, image.width, image.height
                ),
                size(image.png.len() as u64).to_lowercase(),
            ];
            fields.extend(record.source.as_deref().map(str::to_lowercase));
            fields
        }
        Content::Files(files) => files
            .iter()
            .map(|file| file.path.to_string_lossy().to_lowercase())
            .collect(),
    }
}

/// What the history keeps, from `[clipboard]` in config.toml.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    max_entries: usize,
    pub max_entry_bytes: usize,
    pub max_image_bytes: usize,
    pub include_images: bool,
    pub include_files: bool,
    /// Seconds an entry lives; `None` keeps it.
    retention: Option<u64>,
    ignored_apps: Vec<String>,
}

impl Policy {
    /// Apps whose copies are never recorded, which extensions don't see either.
    pub fn ignored_apps(&self) -> &[String] {
        &self.ignored_apps
    }

    fn keeps(&self, content: &Content) -> bool {
        let (allowed, limit) = match content.kind() {
            Kind::Text => (true, self.max_entry_bytes),
            Kind::Image => (self.include_images, self.max_image_bytes),
            Kind::Files => (self.include_files, self.max_entry_bytes),
        };
        allowed && !content.is_blank() && content.bytes() <= limit
    }

    fn expired(&self, copied_at: u64, now: u64) -> bool {
        self.cutoff(now).is_some_and(|cutoff| copied_at < cutoff)
    }

    fn cutoff(&self, now: u64) -> Option<u64> {
        self.retention.map(|seconds| now.saturating_sub(seconds))
    }
}

impl From<&config::Clipboard> for Policy {
    fn from(config: &config::Clipboard) -> Self {
        Self {
            max_entries: config.max_entries,
            max_entry_bytes: config.max_entry_bytes,
            max_image_bytes: config.max_image_bytes,
            include_images: config.include_images,
            include_files: config.include_files,
            retention: config.retention.seconds(),
            ignored_apps: config.ignored_apps.clone(),
        }
    }
}

impl Default for Policy {
    fn default() -> Self {
        (&config::Clipboard::default()).into()
    }
}

pub struct History {
    /// Newest first.
    entries: VecDeque<Arc<Entry>>,
    next_id: u64,
    policy: Policy,
    access: Access,
    store: Box<dyn Store>,
    /// Where a store failure that no caller waits for is reported.
    report: fn(&StoreError),
}

impl Default for History {
    fn default() -> Self {
        Self::new(Policy::default())
    }
}

impl History {
    /// A history kept in memory only.
    pub fn new(policy: Policy) -> Self {
        Self {
            entries: VecDeque::new(),
            next_id: 0,
            policy,
            access: Access::default(),
            store: Box::new(MemoryStore::default()),
            report: |_| {},
        }
    }

    /// A history that keeps what `store` holds and writes every change through to it.
    /// Records the policy no longer keeps are deleted from the store as it loads.
    pub fn with_store(policy: Policy, mut store: Box<dyn Store>, report: fn(&StoreError), now: u64) -> Self {
        let mut records = store.list().unwrap_or_else(|error| {
            report(&error);
            Vec::new()
        });
        records.sort_by(|a, b| b.copied_at.cmp(&a.copied_at).then(b.id.cmp(&a.id)));
        let mut history = Self {
            next_id: records.iter().map(|record| record.id).max().unwrap_or(0),
            entries: records
                .into_iter()
                .map(|record| Arc::new(Entry::new(record)))
                .collect(),
            policy,
            access: Access::default(),
            store,
            report,
        };
        history.enforce(now);
        history
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// Apply new limits, dropping what no longer fits.
    pub fn set_policy(&mut self, policy: Policy, now: u64) {
        self.policy = policy;
        self.enforce(now);
    }

    /// Whether the source app is allowed to contribute to history.
    pub fn accepts_source(&self, source: Option<&str>) -> bool {
        !source.is_some_and(|source| self.policy.ignored_apps.iter().any(|app| app == source))
    }

    /// Record a copy as the newest entry. An equal entry moves to the front instead of
    /// repeating. Blank, oversized and excluded kinds of copies are not recorded.
    pub fn push(&mut self, content: Content, source: Option<String>, now: u64) -> bool {
        if !self.policy.keeps(&content) {
            return false;
        }
        self.evict(|_, entry| !entry.record.content.same(&content));
        self.next_id += 1;
        let record = Arc::new(Record {
            id: self.next_id,
            content,
            copied_at: now,
            source,
        });
        if let Err(error) = self.store.put(&record) {
            (self.report)(&error);
        }
        self.entries.push_front(Arc::new(Entry::new(record)));
        self.enforce(now);
        true
    }

    /// Drop entries past the retention period.
    fn expire(&mut self, now: u64) {
        let Some(cutoff) = self.policy.cutoff(now) else {
            return;
        };
        let before = self.entries.len();
        self.entries.retain(|entry| entry.record.copied_at >= cutoff);
        if self.entries.len() != before
            && let Err(error) = self.store.expire(cutoff)
        {
            (self.report)(&error);
        }
    }

    /// Drop what the policy no longer keeps, oldest first past the limits.
    fn enforce(&mut self, now: u64) {
        self.expire(now);
        let policy = self.policy.clone();
        let mut images = 0;
        self.evict(|index, entry| {
            let content = &entry.record.content;
            if index >= policy.max_entries
                || !policy.keeps(content)
                || policy.expired(entry.record.copied_at, now)
            {
                return false;
            }
            let Content::Image(image) = content else {
                return true;
            };
            let total = images + image.png.len();
            let fits = total <= IMAGE_MEMORY;
            if fits {
                images = total;
            }
            fits
        });
    }

    /// Keep the entries `keep` accepts, newest first, deleting the rest from the store too.
    fn evict(&mut self, mut keep: impl FnMut(usize, &Entry) -> bool) {
        let mut index = 0;
        let mut dropped = Vec::new();
        self.entries.retain(|entry| {
            let kept = keep(index, entry);
            if kept {
                index += 1;
            } else {
                dropped.push(entry.record.id);
            }
            kept
        });
        for id in dropped {
            if let Err(error) = self.store.delete(id) {
                (self.report)(&error);
            }
        }
    }

    /// The entry's record, to copy it back.
    pub fn record(&self, id: u64) -> Option<Arc<Record>> {
        self.entries
            .iter()
            .find(|entry| entry.record.id == id)
            .map(|entry| entry.record.clone())
    }

    pub fn remove(&mut self, id: u64) -> Result<bool, StoreError> {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.record.id != id);
        self.store.delete(id)?;
        Ok(self.entries.len() != before)
    }

    pub fn clear(&mut self) -> Result<(), StoreError> {
        self.entries.clear();
        self.store.clear()
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
            let title = entry.record.content.title();
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

/// Bytes in decimal units, as Finder shows them.
fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 999.95 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

fn item(entry: &Entry, title: String, matched: Vec<usize>, now: u64, exists: fn(&Path) -> bool) -> Item {
    let record = &entry.record;
    let mut parts = vec![ago(now, record.copied_at)];
    let mut available = true;
    let mut reveal = None;
    let (icon, payload) = match &record.content {
        Content::Text(text) => {
            let lines = text.lines().count();
            parts.push(if lines > 1 {
                plural(lines, "line", "lines")
            } else {
                plural(text.chars().count(), "character", "characters")
            });
            (None, Payload::text(text.clone()))
        }
        Content::Image(image) => {
            parts.push(size(image.png.len() as u64));
            parts.extend(record.source.clone());
            (None, Payload::default())
        }
        Content::Files(files) => {
            let present = FileRef::present(files, exists);
            available = !present.is_empty();
            if !available {
                parts.push("Unavailable".into());
            }
            match files.as_slice() {
                [file] => {
                    parts.extend(file.path.parent().map(|parent| tilde(&parent.to_string_lossy())));
                    parts.extend(file.size.filter(|_| available).map(size));
                }
                files => parts.push(plural(files.len(), "file", "files")),
            }
            reveal = present.first().cloned();
            (
                reveal.clone().map(crate::Icon::File),
                Payload {
                    text: None,
                    paths: present,
                },
            )
        }
    };
    let restore = |paste| Action::RestoreClipboardEntry {
        entry: record.id,
        paste,
    };
    let mut actions = Vec::new();
    if available {
        actions.push(ItemAction::new("copy", "Copy", restore(false)));
        actions.push(ItemAction::new("paste", "Paste", restore(true)));
    }
    actions.extend(reveal.map(|path| ItemAction::new("reveal", "Reveal in Finder", Action::Reveal(path))));
    actions.push(
        ItemAction::new(
            "delete",
            "Delete from History",
            Action::DeleteClipboardEntry(record.id),
        )
        .consumes(),
    );
    actions.push(
        ItemAction::new("clear", "Clear History", Action::ClearClipboardHistory)
            .confirm("Clear the whole clipboard history?", "Clear History")
            .consumes(),
    );
    Item {
        id: format!("clipboard-{}", record.id),
        payload,
        view: View::Text {
            title,
            subtitle: parts.join(" · "),
        },
        record_usage: false,
        icon,
        matched,
        completion: None,
        score: None,
        primary: available.then(|| "copy".into()),
        alternate: available.then(|| "paste".into()),
        actions,
    }
}

pub struct ClipboardProvider {
    history: Arc<Mutex<History>>,
    access_item: fn(Access) -> Option<Item>,
    /// Unix seconds; a parameter so tests can fix the time.
    clock: fn() -> u64,
    /// Whether a copied file is still there; a parameter so tests need no files.
    exists: fn(&Path) -> bool,
}

impl ClipboardProvider {
    pub fn new(history: Arc<Mutex<History>>, access_item: fn(Access) -> Option<Item>) -> Self {
        Self {
            history,
            access_item,
            clock: crate::session::unix_now,
            exists: |path| path.exists(),
        }
    }
}

impl Provider for ClipboardProvider {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        let now = (self.clock)();
        let (access, entries) = {
            let mut history = self.history.lock().map_err(|_| ProviderError)?;
            history.expire(now);
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
                .map(|(entry, title, matched)| item(entry, title, matched, now, self.exists)),
        );
        emit(items);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
