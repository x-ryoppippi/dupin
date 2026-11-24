//! The `items()` catalogue Dupin holds for each loaded package revision. It is asked for on
//! first use, after an action invalidates it and once its `ttl` passes, never per keystroke:
//! typing matches the rows held here natively. A package revision Dupin reloads starts a new,
//! empty catalogue with the new [`super::Extension`].
//!
//! Catalogues live in memory only. Rebuilding one costs a call to the package, and one written
//! to disk would keep what a package lists, such as clipboard history, past the package's own
//! say; a package that wants its list to outlast a restart keeps it in its own storage.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant};

use dupin_core::catalogue::{Catalogue, Change, ChangeError, Entry, Limits};
use dupin_core::{Batch, Item};
use dupin_js_runtime::row::{MAX_PAGE_ROWS, PageChange};
use dupin_js_runtime::web_search::WebSearch;
use dupin_js_runtime::{CataloguePage, Diagnostic, Preferences, Reason, Row, RunError};

/// What one package's catalogue may hold.
pub const LIMITS: Limits = Limits {
    rows: 10_000,
    bytes: 8 * 1024 * 1024,
};
/// Text all catalogues together may hold. Past it, the least recently used are dropped and
/// asked for again on their next use.
const BUDGET_BYTES: usize = 24 * 1024 * 1024;
/// Parts one refresh may take: a full catalogue in the largest parts, and one more.
const MAX_PARTS: usize = LIMITS.rows / MAX_PAGE_ROWS + 1;

pub(crate) struct Cached {
    shared: Arc<Shared>,
}

/// Asks the package for the next part of its catalogue, after the cursor it gave.
pub(crate) type Fetch<'a> =
    dyn FnMut(Option<&str>, Reason, &Preferences) -> Result<CataloguePage, RunError> + 'a;

/// How the rows a package sends become the entries Dupin holds. An entry's ID need not be the
/// row's, so the IDs an update removes are mapped the same way before they reach the catalogue.
pub(crate) trait Convert {
    fn entry(&self, row: Row) -> Entry;
    /// The ID of the entry [`Convert::entry`] makes from a row whose ID is `id`.
    fn entry_id(&self, id: &str) -> String;
}

struct Shared {
    state: Mutex<State>,
    /// Kept outside the lock, so the budget can weigh a catalogue another thread is filling.
    bytes: AtomicUsize,
    used: AtomicU64,
}

struct State {
    catalogue: Catalogue,
    cursor: Option<String>,
    expires: Option<Instant>,
    /// Why the package must be asked before the catalogue is used again, if it must.
    due: Option<Reason>,
    /// The preferences the catalogue was asked for with.
    preferences: Option<Arc<Preferences>>,
}

impl Default for Cached {
    fn default() -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                catalogue: Catalogue::default(),
                cursor: None,
                expires: None,
                due: Some(Reason::Load),
                preferences: None,
            }),
            bytes: AtomicUsize::new(0),
            used: AtomicU64::new(0),
        });
        held()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(Arc::downgrade(&shared));
        Self { shared }
    }
}

impl Cached {
    /// Asks the package again before the next use, with the cursor it last gave.
    pub fn invalidate(&self) {
        let mut state = self.shared.lock();
        if state.due.is_none() {
            state.due = Some(Reason::Invalidated);
        }
    }

    /// The rows for `query`, after asking the package through `fetch` if the catalogue is
    /// missing, invalidated, expired or was asked for with other `preferences`. Web searches are
    /// reached by their alias instead, through [`Cached::web_searches`].
    pub fn search(
        &self,
        query: &str,
        limit: usize,
        preferences: &Arc<Preferences>,
        fetch: &mut Fetch<'_>,
        convert: &dyn Convert,
    ) -> Result<Vec<Batch>, RunError> {
        self.fresh(preferences, fetch, convert, |catalogue| {
            catalogue.search(query, limit)
        })
    }

    /// The catalogue's web searches, each with the row it shows before a query is typed,
    /// refreshed as [`Cached::search`] refreshes the catalogue.
    pub fn web_searches(
        &self,
        preferences: &Arc<Preferences>,
        fetch: &mut Fetch<'_>,
        convert: &dyn Convert,
    ) -> Result<Vec<(Item, WebSearch)>, RunError> {
        self.fresh(preferences, fetch, convert, held_web_searches)
    }

    /// The web searches Dupin holds right now, without asking the package or waiting for a
    /// search that is filling the catalogue.
    pub fn held_web_searches(&self) -> Vec<(Item, WebSearch)> {
        match self.shared.state.try_lock() {
            Ok(state) if state.due.is_none() => held_web_searches(&state.catalogue),
            _ => Vec::new(),
        }
    }

    fn fresh<T>(
        &self,
        preferences: &Arc<Preferences>,
        fetch: &mut Fetch<'_>,
        convert: &dyn Convert,
        read: impl FnOnce(&Catalogue) -> T,
    ) -> Result<T, RunError> {
        let mut state = self.shared.lock();
        self.shared.used.store(now_ms(), Ordering::Relaxed);
        let refreshed = state.refresh(Instant::now(), preferences, fetch, convert);
        self.shared
            .bytes
            .store(state.catalogue.bytes(), Ordering::Relaxed);
        refreshed?;
        let read = read(&state.catalogue);
        drop(state);
        self.keep_within_budget();
        Ok(read)
    }

    /// How many rows the catalogue holds.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        let listed = self.shared.lock().catalogue.search("", usize::MAX);
        listed.iter().map(|batch| batch.items.len()).sum()
    }

    /// Drops the least recently used other catalogues while all of them hold more than the
    /// budget. One being filled or searched right now is skipped.
    fn keep_within_budget(&self) {
        let mut held = held().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        held.retain(|shared| shared.strong_count() > 0);
        let mut others: Vec<Arc<Shared>> = held
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|shared| !Arc::ptr_eq(shared, &self.shared))
            .collect();
        drop(held);
        let weight = |shared: &Shared| shared.bytes.load(Ordering::Relaxed);
        let mut total = weight(&self.shared) + others.iter().map(|shared| weight(shared)).sum::<usize>();
        others.sort_by_key(|shared| shared.used.load(Ordering::Relaxed));
        for other in others {
            if total <= BUDGET_BYTES {
                return;
            }
            let Ok(mut state) = other.state.try_lock() else {
                continue;
            };
            total -= weight(&other);
            state.forget();
            other.bytes.store(0, Ordering::Relaxed);
        }
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn held_web_searches(catalogue: &Catalogue) -> Vec<(Item, WebSearch)> {
    catalogue
        .web_searches()
        .map(|(entry, search)| (entry.item.clone(), search.clone()))
        .collect()
}

impl State {
    fn forget(&mut self) {
        self.catalogue = Catalogue::default();
        self.cursor = None;
        self.expires = None;
        self.due = Some(Reason::Load);
        self.preferences = None;
    }

    fn refresh(
        &mut self,
        now: Instant,
        preferences: &Arc<Preferences>,
        fetch: &mut Fetch<'_>,
        convert: &dyn Convert,
    ) -> Result<(), RunError> {
        let expired = self.expires.is_some_and(|expires| now >= expires);
        let changed = self
            .preferences
            .as_ref()
            .is_some_and(|held| !Arc::ptr_eq(held, preferences));
        let Some(mut reason) = changed
            .then_some(Reason::Load)
            .or(self.due)
            .or(expired.then_some(Reason::Expired))
        else {
            return Ok(());
        };
        if reason == Reason::Load {
            self.forget();
        }
        for _ in 0..MAX_PARTS {
            let cursor = if reason == Reason::Load {
                None
            } else {
                self.cursor.clone()
            };
            let page = fetch(cursor.as_deref(), reason, preferences).inspect_err(|_| self.forget())?;
            let change = match page.change {
                PageChange::Reset { rows } => {
                    Change::Reset(rows.into_iter().map(|row| convert.entry(row)).collect())
                }
                PageChange::Update {
                    append,
                    replace,
                    remove,
                } => Change::Update {
                    append: append.into_iter().map(|row| convert.entry(row)).collect(),
                    replace: replace.into_iter().map(|row| convert.entry(row)).collect(),
                    remove: remove.iter().map(|id| convert.entry_id(id)).collect(),
                },
            };
            if let Err(problem) = self.catalogue.apply(change, LIMITS) {
                self.forget();
                return Err(RunError::Failed(rejected(&problem)));
            }
            self.cursor = page.cursor;
            self.expires = page.ttl.map(|seconds| now + Duration::from_secs(seconds.into()));
            if !page.more {
                self.due = None;
                self.preferences = Some(preferences.clone());
                return Ok(());
            }
            reason = Reason::More;
        }
        self.forget();
        Err(RunError::Failed(
            Diagnostic::new(
                "too-many-parts",
                format!("items kept answering `more: true` after {MAX_PARTS} parts"),
            )
            .hint("Set `more` only while rows are left to send."),
        ))
    }
}

fn rejected(problem: &ChangeError) -> Diagnostic {
    let (code, hint) = match problem {
        ChangeError::DuplicateId(_) => (
            "duplicate-id",
            "Give every item a different `id`; to change an item Dupin holds, `replace` it.",
        ),
        ChangeError::UnknownId(_) => (
            "unknown-id",
            "`replace` only items Dupin holds; send new ones with `append`.",
        ),
        ChangeError::TooManyRows { .. } | ChangeError::TooManyBytes { .. } => (
            "catalogue-too-large",
            "List fewer or shorter items; Dupin asks again from the start after this.",
        ),
    };
    Diagnostic::new(code, problem.to_string()).hint(hint)
}

/// Every catalogue alive, for the budget across them.
fn held() -> &'static Mutex<Vec<Weak<Shared>>> {
    static HELD: OnceLock<Mutex<Vec<Weak<Shared>>>> = OnceLock::new();
    HELD.get_or_init(Mutex::default)
}

fn now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use dupin_core::{Item, Payload, View};

    fn row(id: &str, title: &str) -> Row {
        Row {
            id: id.into(),
            title: title.into(),
            subtitle: String::new(),
            keywords: Vec::new(),
            icon: None,
            section: None,
            detail: None,
            actions: Vec::new(),
            web_search: None,
        }
    }

    /// Prefixes IDs as the provider does, so an update that names rows by the package's IDs
    /// only applies when those IDs are mapped too.
    struct Prefixed;

    impl Convert for Prefixed {
        fn entry(&self, row: Row) -> Entry {
            Entry {
                item: Item {
                    id: self.entry_id(&row.id),
                    view: View::Text {
                        title: row.title,
                        subtitle: row.subtitle,
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
                keywords: row.keywords,
                section: None,
                detail: None,
                web_search: row.web_search,
            }
        }

        fn entry_id(&self, id: &str) -> String {
            format!("held/{id}")
        }
    }

    const ENTRY: &Prefixed = &Prefixed;

    fn none() -> Arc<Preferences> {
        static NONE: OnceLock<Arc<Preferences>> = OnceLock::new();
        NONE.get_or_init(Arc::default).clone()
    }

    fn page(change: PageChange, cursor: &str, more: bool) -> CataloguePage {
        CataloguePage {
            change,
            ttl: None,
            cursor: Some(cursor.into()),
            more,
        }
    }

    fn titles(batches: &[Batch]) -> Vec<String> {
        batches
            .iter()
            .flat_map(|batch| &batch.items)
            .map(|item| item.display_title().to_owned())
            .collect()
    }

    #[test]
    fn the_package_is_asked_once_and_typing_only_matches_what_is_held() {
        let cached = Cached::default();
        let mut calls = Vec::new();
        let mut fetch = |cursor: Option<&str>, reason, _: &Preferences| {
            calls.push((cursor.map(str::to_owned), reason));
            Ok(page(
                PageChange::Reset {
                    rows: vec![row("a", "Safari"), row("b", "Notes")],
                },
                "1",
                false,
            ))
        };
        for query in ["", "s", "sa", "saf"] {
            cached.search(query, 10, &none(), &mut fetch, ENTRY).unwrap();
        }
        assert_eq!(
            titles(&cached.search("no", 10, &none(), &mut fetch, ENTRY).unwrap()),
            ["Notes"]
        );
        assert_eq!(calls, [(None, Reason::Load)]);
    }

    #[test]
    fn parts_and_updates_follow_the_cursor_and_invalidation_asks_with_it() {
        let cached = Cached::default();
        let mut calls = Vec::new();
        let mut fetch = |cursor: Option<&str>, reason, _: &Preferences| {
            calls.push((cursor.map(str::to_owned), reason));
            Ok(match (cursor, reason) {
                (None, Reason::Load) => page(
                    PageChange::Reset {
                        rows: vec![row("a", "Alpha")],
                    },
                    "1",
                    true,
                ),
                (Some("1"), Reason::More) => page(
                    PageChange::Update {
                        append: vec![row("b", "Beta")],
                        replace: Vec::new(),
                        remove: Vec::new(),
                    },
                    "2",
                    false,
                ),
                (Some("2"), Reason::Invalidated) => page(
                    PageChange::Update {
                        append: Vec::new(),
                        replace: vec![row("b", "Bravo")],
                        remove: vec!["a".into()],
                    },
                    "3",
                    false,
                ),
                other => panic!("unexpected call {other:?}"),
            })
        };
        assert_eq!(
            titles(&cached.search("", 10, &none(), &mut fetch, ENTRY).unwrap()),
            ["Alpha", "Beta"]
        );
        cached.invalidate();
        assert_eq!(
            titles(&cached.search("", 10, &none(), &mut fetch, ENTRY).unwrap()),
            ["Bravo"]
        );
        assert_eq!(calls.len(), 3);
    }

    #[test]
    fn a_removed_row_is_gone_and_its_id_can_come_back() {
        let cached = Cached::default();
        let mut answers = vec![
            PageChange::Reset {
                rows: vec![row("clip-1", "First"), row("clip-2", "Second")],
            },
            PageChange::Update {
                append: Vec::new(),
                replace: Vec::new(),
                remove: vec!["clip-1".into()],
            },
            PageChange::Update {
                append: vec![row("clip-1", "Again")],
                replace: Vec::new(),
                remove: Vec::new(),
            },
        ]
        .into_iter();
        let mut fetch = |_: Option<&str>, _, _: &Preferences| {
            Ok(page(answers.next().expect("asked once per change"), "1", false))
        };
        assert_eq!(
            titles(&cached.search("", 10, &none(), &mut fetch, ENTRY).unwrap()),
            ["First", "Second"]
        );
        cached.invalidate();
        assert_eq!(
            titles(&cached.search("", 10, &none(), &mut fetch, ENTRY).unwrap()),
            ["Second"]
        );
        cached.invalidate();
        assert_eq!(
            titles(&cached.search("", 10, &none(), &mut fetch, ENTRY).unwrap()),
            ["Second", "Again"]
        );
    }

    #[test]
    fn an_expired_catalogue_is_asked_again_and_a_failure_starts_over() {
        let cached = Cached::default();
        let mut reasons = Vec::new();
        let mut fetch = |_: Option<&str>, reason, _: &Preferences| {
            reasons.push(reason);
            match reasons.len() {
                1 => Ok(CataloguePage {
                    ttl: Some(1),
                    ..page(
                        PageChange::Reset {
                            rows: vec![row("a", "A")],
                        },
                        "1",
                        false,
                    )
                }),
                2 => Err(RunError::Failed(Diagnostic::new("items-threw", "boom"))),
                _ => Ok(page(
                    PageChange::Reset {
                        rows: vec![row("b", "B")],
                    },
                    "1",
                    false,
                )),
            }
        };
        cached.search("", 10, &none(), &mut fetch, ENTRY).unwrap();
        cached.shared.lock().expires = Some(Instant::now());
        assert!(cached.search("", 10, &none(), &mut fetch, ENTRY).is_err());
        assert_eq!(cached.len(), 0);
        assert_eq!(
            titles(&cached.search("", 10, &none(), &mut fetch, ENTRY).unwrap()),
            ["B"]
        );
        assert_eq!(reasons, [Reason::Load, Reason::Expired, Reason::Load]);
    }

    #[test]
    fn a_rejected_change_or_endless_parts_drop_the_catalogue() {
        let cached = Cached::default();
        let mut fetch = |_: Option<&str>, _, _: &Preferences| {
            Ok(page(
                PageChange::Update {
                    append: Vec::new(),
                    replace: vec![row("missing", "M")],
                    remove: Vec::new(),
                },
                "1",
                false,
            ))
        };
        let error = cached.search("", 10, &none(), &mut fetch, ENTRY).unwrap_err();
        assert!(
            matches!(&error, RunError::Failed(diagnostic) if diagnostic.code == "unknown-id"),
            "{error:?}"
        );
        let mut endless =
            |_: Option<&str>, _, _: &Preferences| Ok(page(PageChange::Reset { rows: Vec::new() }, "1", true));
        let error = cached.search("", 10, &none(), &mut endless, ENTRY).unwrap_err();
        assert!(
            matches!(&error, RunError::Failed(diagnostic) if diagnostic.code == "too-many-parts"),
            "{error:?}"
        );
    }

    #[test]
    fn other_preferences_load_the_catalogue_again_and_web_searches_come_from_it() {
        use dupin_js_runtime::web_search::WebSearch;

        let cached = Cached::default();
        let mut seen = Vec::new();
        let mut fetch = |_: Option<&str>, reason, preferences: &Preferences| {
            seen.push((reason, preferences.len()));
            let mut search = row("g", "Search Google");
            search.web_search = Some(WebSearch {
                alias: "g".into(),
                url: "https://www.google.com/search?q={query}".into(),
                title: "Search Google for {query}".into(),
                fallback: None,
            });
            Ok(page(
                PageChange::Reset {
                    rows: vec![search, row("a", "Alpha")],
                },
                "1",
                false,
            ))
        };
        assert!(cached.held_web_searches().is_empty());
        let first = none();
        let searches = cached.web_searches(&first, &mut fetch, ENTRY).unwrap();
        assert_eq!(searches.len(), 1);
        assert_eq!(searches[0].0.id, "held/g");
        assert_eq!(searches[0].1.alias, "g");
        assert_eq!(cached.held_web_searches().len(), 1);
        assert_eq!(
            titles(&cached.search("", 10, &first, &mut fetch, ENTRY).unwrap()),
            ["Alpha"]
        );
        let mut changed = Preferences::new();
        changed.insert("webFallbacks".into(), serde_json::json!(["g"]));
        cached
            .web_searches(&Arc::new(changed), &mut fetch, ENTRY)
            .unwrap();
        assert_eq!(seen, [(Reason::Load, 0), (Reason::Load, 1)]);
    }
}
