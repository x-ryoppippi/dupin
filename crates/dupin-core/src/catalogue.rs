//! A list of rows kept in memory and matched natively as the user types, for a source that
//! hands over its whole list rather than answering each query: an extension's `items()`, and
//! built-ins ported to the same shape. Matching folds every name once, when the row arrives, so
//! a keystroke costs one pass of the matcher over the list and no call into the source.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use dupin_plugin_protocol::web_search::WebSearch;

use crate::fuzzy::{self, Name, Query, Scratch};
use crate::{Action, Batch, Detail, Item};

/// A match on a keyword ranks below the same match on the title, as an app's alias does.
const KEYWORD_PENALTY: i32 = 8;

/// One row of a catalogue: what the list shows, and what else matching and presentation need.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Its `id` is unique within the catalogue.
    pub item: Item,
    /// Further words the query is matched against.
    pub keywords: Vec<String>,
    /// The heading the row is listed under while the query is empty.
    pub section: Option<Arc<str>>,
    /// Shown while the row is selected.
    pub detail: Option<Arc<Detail>>,
    /// Makes the row a web search: it is reached by its alias, never matched by the query.
    pub web_search: Option<WebSearch>,
    /// More text [`Matching::Words`] finds the row by, which the row never shows.
    pub match_text: Option<String>,
    /// Unix seconds the row is from; its subtitle starts with how long ago that was.
    pub time: Option<u64>,
    /// The file the row stands for. While it is gone, the row shows as unavailable, without
    /// its icon and the actions in `file_actions`.
    pub file: Option<std::path::PathBuf>,
    pub file_actions: Vec<String>,
}

/// How a catalogue matches what is typed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Matching {
    /// Ranked as apps are, by how well the title or a keyword matches.
    #[default]
    Fuzzy,
    /// Every word, ignoring case, anywhere in the title, a keyword or the match text, with the
    /// rows kept in the catalogue's own order, as a history is searched.
    Words,
}

/// How much one catalogue may hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub rows: usize,
    /// Bytes of text across every row.
    pub bytes: usize,
}

/// A whole new list, or changes to the current one by row ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Reset(Vec<Entry>),
    Update {
        /// New rows, before the existing ones, in their own order.
        prepend: Vec<Entry>,
        /// New rows, after the existing ones.
        append: Vec<Entry>,
        /// Rows that take the place of the existing rows with their IDs.
        replace: Vec<Entry>,
        /// IDs of rows to drop. An ID the catalogue doesn't hold is ignored.
        remove: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeError {
    /// Two rows would share this ID.
    DuplicateId(String),
    /// A replacement names a row the catalogue doesn't hold.
    UnknownId(String),
    TooManyRows {
        rows: usize,
        limit: usize,
    },
    TooManyBytes {
        bytes: usize,
        limit: usize,
    },
}

impl fmt::Display for ChangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateId(id) => write!(f, "`{id}` is the id of more than one item"),
            Self::UnknownId(id) => write!(f, "`{id}` replaces an item the catalogue doesn't hold"),
            Self::TooManyRows { rows, limit } => {
                write!(
                    f,
                    "the catalogue would hold {rows} items; at most {limit} are allowed"
                )
            }
            Self::TooManyBytes { bytes, limit } => write!(
                f,
                "the catalogue would hold {} KiB of text; at most {} KiB are allowed",
                bytes.div_ceil(1024),
                limit / 1024
            ),
        }
    }
}

impl std::error::Error for ChangeError {}

#[derive(Debug, Default)]
pub struct Catalogue {
    rows: Vec<Row>,
    index: HashMap<String, usize>,
    bytes: usize,
    matching: Matching,
}

#[derive(Debug)]
struct Row {
    entry: Entry,
    title: Name,
    keywords: Vec<Name>,
    /// For [`Matching::Words`]: the title, keywords and match text, lowercased once. The match
    /// text is kept only here.
    folded: String,
    bytes: usize,
}

impl Row {
    fn new(mut entry: Entry, matching: Matching) -> Self {
        let bytes = entry_bytes(&entry);
        let match_text = entry.match_text.take();
        match matching {
            Matching::Fuzzy => Self {
                title: Name::new(entry.item.display_title()),
                keywords: entry.keywords.iter().map(|keyword| Name::new(keyword)).collect(),
                folded: String::new(),
                entry,
                bytes,
            },
            Matching::Words => {
                let mut folded = entry.item.display_title().to_lowercase();
                for text in entry.keywords.iter().chain(&match_text) {
                    folded.push('\n');
                    folded.push_str(&text.to_lowercase());
                }
                Self {
                    title: Name::new(""),
                    keywords: Vec::new(),
                    folded,
                    entry,
                    bytes,
                }
            }
        }
    }

    /// The row's best match, and whether it was on its title rather than one of its keywords.
    fn score(&self, query: &Query, scratch: &mut Scratch) -> Option<(i32, bool)> {
        let on_title = fuzzy::score_only(query, &self.title, scratch);
        let on_keyword = self
            .keywords
            .iter()
            .filter_map(|keyword| fuzzy::score_only(query, keyword, scratch))
            .max()
            .map(|score| score - KEYWORD_PENALTY);
        match (on_title, on_keyword) {
            (Some(title), Some(keyword)) if keyword > title => Some((keyword, false)),
            (Some(title), _) => Some((title, true)),
            (None, keyword) => keyword.map(|score| (score, false)),
        }
    }
}

impl Catalogue {
    pub fn new(matching: Matching) -> Self {
        Self {
            matching,
            ..Self::default()
        }
    }

    pub fn matching(&self) -> Matching {
        self.matching
    }

    /// Bytes of text the catalogue holds.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    fn contains(&self, id: &str) -> bool {
        self.index.contains_key(id)
    }

    /// Applies `change` whole, or not at all when it would break `limits` or the rows' IDs.
    pub fn apply(&mut self, change: Change, limits: Limits) -> Result<(), ChangeError> {
        match change {
            Change::Reset(entries) => {
                let mut ids = HashSet::with_capacity(entries.len());
                if let Some(entry) = entries.iter().find(|entry| !ids.insert(entry.item.id.as_str())) {
                    return Err(ChangeError::DuplicateId(entry.item.id.clone()));
                }
                check_rows(entries.len(), limits)?;
                let matching = self.matching;
                let rows: Vec<Row> = entries
                    .into_iter()
                    .map(|entry| Row::new(entry, matching))
                    .collect();
                let bytes = rows.iter().map(|row| row.bytes).sum();
                check_bytes(bytes, limits)?;
                self.rows = rows;
                self.bytes = bytes;
            }
            Change::Update {
                prepend,
                append,
                replace,
                remove,
            } => {
                let remove: HashSet<String> = remove.into_iter().filter(|id| self.contains(id)).collect();
                let mut replaced = HashSet::with_capacity(replace.len());
                for entry in &replace {
                    let id = &entry.item.id;
                    if !self.contains(id) || remove.contains(id) {
                        return Err(ChangeError::UnknownId(id.clone()));
                    }
                    if !replaced.insert(id.as_str()) {
                        return Err(ChangeError::DuplicateId(id.clone()));
                    }
                }
                let mut appended = HashSet::with_capacity(prepend.len() + append.len());
                for entry in prepend.iter().chain(&append) {
                    let id = entry.item.id.as_str();
                    let held = self.contains(id) && !remove.contains(id);
                    if held || !appended.insert(id) {
                        return Err(ChangeError::DuplicateId(id.to_owned()));
                    }
                }
                check_rows(
                    self.rows.len() - remove.len() + prepend.len() + append.len(),
                    limits,
                )?;
                let matching = self.matching;
                let row = |entry| Row::new(entry, matching);
                let replace: Vec<Row> = replace.into_iter().map(row).collect();
                let prepend: Vec<Row> = prepend.into_iter().map(row).collect();
                let append: Vec<Row> = append.into_iter().map(row).collect();
                let dropped: usize = remove.iter().map(|id| self.rows[self.index[id]].bytes).sum();
                let swapped_out: usize = replace
                    .iter()
                    .map(|row| self.rows[self.index[&row.entry.item.id]].bytes)
                    .sum();
                let added: usize = replace
                    .iter()
                    .chain(&prepend)
                    .chain(&append)
                    .map(|row| row.bytes)
                    .sum();
                let bytes = self.bytes - dropped - swapped_out + added;
                check_bytes(bytes, limits)?;
                for row in replace {
                    let at = self.index[&row.entry.item.id];
                    self.rows[at] = row;
                }
                if !remove.is_empty() {
                    self.rows.retain(|row| !remove.contains(&row.entry.item.id));
                }
                if !prepend.is_empty() {
                    self.rows.splice(0..0, prepend);
                }
                self.rows.extend(append);
                self.bytes = bytes;
            }
        }
        self.index = self
            .rows
            .iter()
            .enumerate()
            .map(|(at, row)| (row.entry.item.id.clone(), at))
            .collect();
        Ok(())
    }

    /// The rows for `query`, at most `limit`, as they show at `now` (Unix seconds). An empty
    /// query lists rows in the catalogue's own order under their sections, without scores, so
    /// ranking leaves them in place. Otherwise [`Matching::Fuzzy`] ranks rows by how well they
    /// match, as apps are, in one batch without sections so usage ranking can reorder them
    /// freely, and [`Matching::Words`] keeps the rows that match in the catalogue's order.
    pub fn search(&self, query: &str, limit: usize, now: u64) -> Vec<Batch> {
        if self.matching == Matching::Words {
            return self.words(query, limit, now);
        }
        let Some(query) = Query::new(query) else {
            return self.listed(limit, now);
        };
        let mut scratch = Scratch::default();
        let mut hits: Vec<(i32, usize, bool)> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.entry.web_search.is_none())
            .filter_map(|(at, row)| {
                row.score(&query, &mut scratch)
                    .map(|(score, on_title)| (score, at, on_title))
            })
            .collect();
        hits.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        hits.truncate(limit);
        let mut batch = Batch {
            details_when_selected: true,
            ..Batch::default()
        };
        for (score, at, on_title) in hits {
            let row = &self.rows[at];
            // Only the rows shown need the characters to highlight.
            let positions = on_title
                .then(|| fuzzy::score_prepared(&query, &row.title))
                .flatten()
                .map(|found| found.positions)
                .unwrap_or_default();
            let entry = &row.entry;
            if let Some(detail) = &entry.detail {
                batch.details.insert(entry.item.id.clone(), detail.clone());
            }
            batch.items.push(Item {
                score: Some(score),
                matched: positions,
                ..shown(entry, now)
            });
        }
        if batch.items.is_empty() {
            return Vec::new();
        }
        vec![batch]
    }

    /// The rows that are web searches, in the catalogue's order.
    pub fn web_searches(&self) -> impl Iterator<Item = (&Entry, &WebSearch)> {
        self.rows
            .iter()
            .filter_map(|row| Some((&row.entry, row.entry.web_search.as_ref()?)))
    }

    fn words(&self, query: &str, limit: usize, now: u64) -> Vec<Batch> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() {
            return self.listed(limit, now);
        }
        let mut batch = Batch {
            details_when_selected: true,
            ..Batch::default()
        };
        let hits = self
            .rows
            .iter()
            .filter(|row| row.entry.web_search.is_none())
            .filter(|row| words.iter().all(|word| row.folded.contains(word.as_str())))
            .take(limit);
        for row in hits {
            let entry = &row.entry;
            if let Some(detail) = &entry.detail {
                batch.details.insert(entry.item.id.clone(), detail.clone());
            }
            batch.items.push(Item {
                score: None,
                matched: highlights(entry.item.display_title(), &words),
                ..shown(entry, now)
            });
        }
        if batch.items.is_empty() {
            return Vec::new();
        }
        vec![batch]
    }

    fn listed(&self, limit: usize, now: u64) -> Vec<Batch> {
        let mut batches: Vec<Batch> = Vec::new();
        let listed = self.rows.iter().filter(|row| row.entry.web_search.is_none());
        for row in listed.take(limit) {
            let entry = &row.entry;
            let section = entry.section.as_deref().map(str::to_owned);
            let batch = match batches.last_mut() {
                Some(batch) if batch.section == section => batch,
                _ => {
                    batches.push(Batch {
                        section,
                        details_when_selected: true,
                        ..Batch::default()
                    });
                    batches.last_mut().expect("just pushed")
                }
            };
            if let Some(detail) = &entry.detail {
                batch.details.insert(entry.item.id.clone(), detail.clone());
            }
            batch.items.push(Item {
                score: None,
                matched: Vec::new(),
                ..shown(entry, now)
            });
        }
        batches
    }
}

/// The row's item as it shows at `now`: with a time, its subtitle starts with how long ago
/// that was, and a row whose file is gone says so and offers only what needs no file.
fn shown(entry: &Entry, now: u64) -> Item {
    let mut item = entry.item.clone();
    let gone = entry.file.as_deref().is_some_and(|file| !file.exists());
    let crate::View::Text { subtitle, .. } = &mut item.view else {
        return item;
    };
    let parts = [
        entry.time.map(|time| ago(now, time)),
        gone.then(|| "Unavailable".to_owned()),
        (!subtitle.is_empty()).then(|| std::mem::take(subtitle)),
    ];
    *subtitle = parts.into_iter().flatten().collect::<Vec<_>>().join(" · ");
    if gone {
        item.icon = None;
        item.actions
            .retain(|action| !entry.file_actions.contains(&action.id));
        let offered = |id: &Option<String>| {
            id.as_ref()
                .filter(|id| item.actions.iter().any(|action| &action.id == *id))
                .cloned()
        };
        item.primary = offered(&item.primary);
        item.alternate = offered(&item.alternate);
    }
    item
}

/// How long before `now` the Unix second `then` was.
fn ago(now: u64, then: u64) -> String {
    let seconds = now.saturating_sub(then);
    match seconds {
        0..60 => "Just now".into(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..86_400 => format!("{} h ago", seconds / 3600),
        _ => format!("{} d ago", seconds / 86_400),
    }
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

fn check_rows(rows: usize, limits: Limits) -> Result<(), ChangeError> {
    if rows > limits.rows {
        return Err(ChangeError::TooManyRows {
            rows,
            limit: limits.rows,
        });
    }
    Ok(())
}

fn check_bytes(bytes: usize, limits: Limits) -> Result<(), ChangeError> {
    if bytes > limits.bytes {
        return Err(ChangeError::TooManyBytes {
            bytes,
            limit: limits.bytes,
        });
    }
    Ok(())
}

/// The text a row holds, counted toward its catalogue's budget.
fn entry_bytes(entry: &Entry) -> usize {
    let item = &entry.item;
    let view = match &item.view {
        crate::View::Text { title, subtitle } => title.len() + subtitle.len(),
        crate::View::Detail { title, body } => title.len() + body.len(),
        crate::View::Pair { source, result } => source.len() + result.len(),
    };
    let actions: usize = item
        .actions
        .iter()
        .map(|action| action.id.len() + action.title.len() + action_bytes(&action.action))
        .sum();
    item.id.len()
        + view
        + item.payload.text.as_ref().map_or(0, String::len)
        + actions
        + entry.keywords.iter().map(String::len).sum::<usize>()
        + entry.section.as_ref().map_or(0, |section| section.len())
        + entry.detail.as_ref().map_or(0, |detail| detail.text_bytes())
        + entry.web_search.as_ref().map_or(0, WebSearch::text_bytes)
        + entry.match_text.as_ref().map_or(0, String::len)
        + entry.file.as_ref().map_or(0, |file| file.as_os_str().len())
}

/// Every action names each of its variants, so one added later must say what it holds.
fn action_bytes(action: &Action) -> usize {
    use crate::ExtensionRequest;
    let path = |path: &std::path::Path| path.as_os_str().len();
    let paths = |paths: &[std::path::PathBuf]| paths.iter().map(|p| path(p)).sum::<usize>();
    match action {
        Action::Open(p)
        | Action::ChooseApplication(p)
        | Action::Trash(p)
        | Action::Browse(p)
        | Action::Rename(p)
        | Action::Duplicate(p)
        | Action::Compress(p)
        | Action::Reveal(p)
        | Action::Preview(p)
        | Action::ScriptArguments(p) => path(p),
        Action::OpenWith { path: p, application } => path(p) + path(application),
        Action::ChooseDestination { path: p, .. } => path(p),
        Action::OpenUrl(text) | Action::Copy(text) | Action::Snippet { text, .. } => text.len(),
        Action::Share {
            target,
            paths: shared,
        } => {
            paths(shared)
                + match target {
                    crate::ShareTarget::Picker => 0,
                    crate::ShareTarget::Service { name, label } => name.len() + label.len(),
                }
        }
        Action::CopyFiles(shared) => paths(shared),
        Action::Push(page) => {
            page.view.len()
                + page.title.len()
                + page
                    .context
                    .as_ref()
                    .map_or(0, |context| context.to_string().len())
        }
        Action::RunScript(run) => {
            path(&run.path) + run.title.len() + run.args.iter().map(String::len).sum::<usize>()
        }
        Action::Extension { extension, request } => {
            extension.len()
                + match request {
                    ExtensionRequest::Run { action, data } => {
                        action.len() + data.as_ref().map_or(0, |data| data.to_string().len())
                    }
                    ExtensionRequest::Paste { text } => text.len(),
                    ExtensionRequest::Allow { scope, revision } => scope.len() + revision.len(),
                    ExtensionRequest::Deny { scope } | ExtensionRequest::RequestAccess { scope } => {
                        scope.len()
                    }
                    ExtensionRequest::TurnOn | ExtensionRequest::Review => 0,
                }
        }
        Action::System(_) | Action::RequestClipboardAccess | Action::OpenPreferences => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Payload, View};

    const LIMITS: Limits = Limits {
        rows: 100,
        bytes: 64 * 1024,
    };

    fn entry(id: &str, title: &str) -> Entry {
        Entry {
            item: Item {
                id: id.into(),
                view: View::Text {
                    title: title.into(),
                    subtitle: String::new(),
                },
                payload: Payload::default(),
                record_usage: true,
                icon: None,
                matched: Vec::new(),
                completion: None,
                score: None,
                primary: None,
                alternate: None,
                actions: Vec::new(),
            },
            keywords: Vec::new(),
            section: None,
            detail: None,
            web_search: None,
            match_text: None,
            time: None,
            file: None,
            file_actions: Vec::new(),
        }
    }

    fn titles(batches: &[Batch]) -> Vec<&str> {
        batches
            .iter()
            .flat_map(|batch| &batch.items)
            .map(Item::display_title)
            .collect()
    }

    fn catalogue(entries: Vec<Entry>) -> Catalogue {
        let mut catalogue = Catalogue::default();
        catalogue.apply(Change::Reset(entries), LIMITS).unwrap();
        catalogue
    }

    #[test]
    fn typing_ranks_rows_as_apps_are_ranked_and_highlights_title_matches() {
        let catalogue = catalogue(vec![
            entry("a", "Hush Safe"),
            entry("b", "Safari Technology Preview"),
            entry("c", "Safari"),
            entry("d", "Notes"),
        ]);
        let batches = catalogue.search("saf", 10, 0);
        assert_eq!(
            titles(&batches),
            ["Safari", "Safari Technology Preview", "Hush Safe"]
        );
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].section, None);
        assert_eq!(batches[0].items[0].matched, [0, 1, 2]);
        assert!(batches[0].items.iter().all(|item| item.score.is_some()));
        assert_eq!(titles(&catalogue.search("saf", 1, 0)), ["Safari"]);
        assert!(catalogue.search("zzz", 10, 0).is_empty());
    }

    fn words(entries: Vec<Entry>) -> Catalogue {
        let mut catalogue = Catalogue::new(Matching::Words);
        catalogue.apply(Change::Reset(entries), LIMITS).unwrap();
        catalogue
    }

    #[test]
    fn words_find_every_word_anywhere_in_order_with_title_highlights() {
        let mut long = entry("a", "Meeting moved");
        long.match_text = Some("Meeting moved to Thursday.\nRoom: Baker Street 221B".into());
        let mut file = entry("b", "Report.pdf");
        file.keywords = vec!["/Users/Shared/Report.pdf".into()];
        let catalogue = words(vec![entry("c", "Baker's dozen"), long, file]);
        let batches = catalogue.search("BAKER room", 10, 0);
        assert_eq!(titles(&batches), ["Meeting moved"]);
        assert!(batches[0].items[0].score.is_none());
        assert!(
            batches[0].items[0].matched.is_empty(),
            "the words are past the title"
        );
        assert_eq!(
            titles(&catalogue.search("baker", 10, 0)),
            ["Baker's dozen", "Meeting moved"]
        );
        assert_eq!(catalogue.search("bak", 1, 0)[0].items[0].matched, [0, 1, 2]);
        assert_eq!(titles(&catalogue.search("shared", 10, 0)), ["Report.pdf"]);
        assert_eq!(catalogue.search("", 10, 0)[0].items.len(), 3);
        assert_eq!(
            catalogue.bytes(),
            "cBaker's dozen".len()
                + "aMeeting moved".len()
                + "Meeting moved to Thursday.\nRoom: Baker Street 221B".len()
                + "bReport.pdf/Users/Shared/Report.pdf".len()
        );
    }

    #[test]
    fn a_timed_row_says_how_long_ago_it_was_when_it_shows() {
        let mut timed = entry("a", "Copied");
        timed.item.view = crate::View::Text {
            title: "Copied".into(),
            subtitle: "12 characters".into(),
        };
        timed.time = Some(1_000);
        let mut bare = entry("b", "Bare");
        bare.time = Some(1_000);
        let catalogue = words(vec![timed, bare]);
        let subtitles = |now| {
            catalogue
                .search("", 10, now)
                .remove(0)
                .items
                .into_iter()
                .map(|item| match item.view {
                    crate::View::Text { subtitle, .. } => subtitle,
                    _ => unreachable!(),
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(subtitles(1_030), ["Just now · 12 characters", "Just now"]);
        assert_eq!(subtitles(1_000 + 7_200), ["2 h ago · 12 characters", "2 h ago"]);
    }

    #[test]
    fn web_searches_are_held_and_updated_by_id_but_never_matched_or_listed() {
        let search = |alias: &str| {
            let mut entry = entry(alias, "Search Safari");
            entry.web_search = Some(WebSearch {
                alias: alias.into(),
                url: "https://example.com/?q={query}".into(),
                title: "Search Safari for {query}".into(),
                fallback: None,
            });
            entry
        };
        let mut catalogue = catalogue(vec![search("s"), entry("b", "Safari"), search("t")]);
        assert_eq!(titles(&catalogue.search("saf", 10, 0)), ["Safari"]);
        assert_eq!(titles(&catalogue.search("", 10, 0)), ["Safari"]);
        let by_words = words(vec![search("s"), entry("b", "Safari")]);
        assert_eq!(titles(&by_words.search("safari", 10, 0)), ["Safari"]);
        let aliases = |catalogue: &Catalogue| {
            catalogue
                .web_searches()
                .map(|(entry, search)| (entry.item.id.clone(), search.alias.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            aliases(&catalogue),
            [("s".into(), "s".into()), ("t".into(), "t".into())]
        );
        catalogue
            .apply(
                Change::Update {
                    prepend: Vec::new(),
                    append: Vec::new(),
                    replace: vec![entry("s", "Plain")],
                    remove: vec!["t".into()],
                },
                LIMITS,
            )
            .unwrap();
        assert!(aliases(&catalogue).is_empty());
        assert_eq!(titles(&catalogue.search("", 10, 0)), ["Plain", "Safari"]);
    }

    #[test]
    fn keywords_match_below_an_equal_title_match_without_highlights() {
        let mut aliased = entry("aliased", "Visual Studio Code");
        aliased.keywords = vec!["editor".into()];
        let catalogue = catalogue(vec![entry("plain", "Editorial"), aliased]);
        let batches = catalogue.search("editor", 10, 0);
        assert_eq!(titles(&batches), ["Editorial", "Visual Studio Code"]);
        assert!(batches[0].items[1].matched.is_empty());
    }

    #[test]
    fn an_empty_query_lists_rows_in_order_under_their_sections_without_scores() {
        let mut first = entry("1", "One");
        first.section = Some("Pinned".into());
        let mut second = entry("2", "Two");
        second.section = Some("Pinned".into());
        second.detail = Some(Arc::new(Detail::new("Two", "**bold**")));
        let third = entry("3", "Three");
        let batches = catalogue(vec![first, second, third]).search("  ", 10, 0);
        assert_eq!(
            batches
                .iter()
                .map(|batch| (batch.section.as_deref(), batch.items.len()))
                .collect::<Vec<_>>(),
            [(Some("Pinned"), 2), (None, 1)]
        );
        assert!(batches.iter().all(|batch| batch.details_when_selected));
        assert!(batches[0].details.contains_key("2"));
        assert!(
            batches
                .iter()
                .flat_map(|batch| &batch.items)
                .all(|item| item.score.is_none())
        );
    }

    #[test]
    fn updates_append_replace_and_remove_by_id_in_place() {
        let mut catalogue = catalogue(vec![entry("a", "Alpha"), entry("b", "Beta"), entry("c", "Gamma")]);
        catalogue
            .apply(
                Change::Update {
                    prepend: vec![],
                    append: vec![entry("d", "Delta")],
                    replace: vec![entry("b", "Bravo")],
                    remove: vec!["a".into(), "never-held".into()],
                },
                LIMITS,
            )
            .unwrap();
        assert_eq!(titles(&catalogue.search("", 10, 0)), ["Bravo", "Gamma", "Delta"]);
        // A removed ID may come back in the same update.
        catalogue
            .apply(
                Change::Update {
                    prepend: vec![],
                    append: vec![entry("c", "Charlie")],
                    replace: vec![],
                    remove: vec!["c".into()],
                },
                LIMITS,
            )
            .unwrap();
        assert_eq!(
            titles(&catalogue.search("", 10, 0)),
            ["Bravo", "Delta", "Charlie"]
        );
        // So it may at the front, as a history moves a copy made again to the top.
        catalogue
            .apply(
                Change::Update {
                    prepend: vec![entry("c", "Charlie again"), entry("e", "Echo")],
                    append: vec![],
                    replace: vec![],
                    remove: vec!["c".into()],
                },
                LIMITS,
            )
            .unwrap();
        assert_eq!(
            titles(&catalogue.search("", 10, 0)),
            ["Charlie again", "Echo", "Bravo", "Delta"]
        );
        let duplicate = Change::Update {
            prepend: vec![entry("e", "Echo twice")],
            append: vec![],
            replace: vec![],
            remove: vec![],
        };
        assert_eq!(
            catalogue.apply(duplicate, LIMITS),
            Err(ChangeError::DuplicateId("e".into()))
        );
    }

    #[test]
    fn a_change_that_breaks_ids_or_limits_leaves_the_catalogue_untouched() {
        let mut catalogue = catalogue(vec![entry("a", "Alpha"), entry("b", "Beta")]);
        let update = |append: Vec<Entry>, replace: Vec<Entry>| Change::Update {
            prepend: Vec::new(),
            append,
            replace,
            remove: vec!["a".into()],
        };
        for (change, error) in [
            (
                update(vec![entry("b", "Again")], vec![]),
                ChangeError::DuplicateId("b".into()),
            ),
            (
                update(vec![], vec![entry("a", "Removed")]),
                ChangeError::UnknownId("a".into()),
            ),
            (
                update(vec![], vec![entry("z", "Missing")]),
                ChangeError::UnknownId("z".into()),
            ),
            (
                Change::Reset(vec![entry("x", "One"), entry("x", "Two")]),
                ChangeError::DuplicateId("x".into()),
            ),
        ] {
            assert_eq!(catalogue.apply(change, LIMITS), Err(error));
        }
        let small = Limits { rows: 2, bytes: 1024 };
        assert_eq!(
            catalogue.apply(update(vec![entry("c", "C"), entry("d", "D")], vec![]), small),
            Err(ChangeError::TooManyRows { rows: 3, limit: 2 })
        );
        let tight = Limits { rows: 10, bytes: 12 };
        assert_eq!(
            catalogue.apply(update(vec![], vec![entry("b", "A much longer title")]), tight),
            Err(ChangeError::TooManyBytes { bytes: 20, limit: 12 })
        );
        assert_eq!(titles(&catalogue.search("", 10, 0)), ["Alpha", "Beta"]);
        assert_eq!(catalogue.bytes(), "aAlphabBeta".len());
    }

    #[test]
    fn the_text_every_action_carries_counts_toward_the_budget() {
        let with = |action: Action| {
            let mut row = entry("a", "A");
            row.item.actions = vec![crate::ItemAction::new("p", "P", action)];
            row
        };
        let text = "x".repeat(16 * 1024);
        let paste = Action::Extension {
            extension: "emoji".into(),
            request: crate::ExtensionRequest::Paste { text: text.clone() },
        };
        let overhead = "aA".len() + "pP".len();
        for (action, held) in [
            (paste, "emoji".len() + text.len()),
            (Action::Copy(text.clone()), text.len()),
            (
                Action::Snippet {
                    text: text.clone(),
                    paste: true,
                },
                text.len(),
            ),
        ] {
            let mut catalogue = Catalogue::default();
            catalogue
                .apply(Change::Reset(vec![with(action.clone())]), LIMITS)
                .unwrap();
            assert_eq!(catalogue.bytes(), overhead + held, "{action:?}");
            let tight = Limits {
                rows: 10,
                bytes: text.len(),
            };
            assert!(
                matches!(
                    Catalogue::default().apply(Change::Reset(vec![with(action.clone())]), tight),
                    Err(ChangeError::TooManyBytes { .. })
                ),
                "{action:?}"
            );
        }
    }
}
