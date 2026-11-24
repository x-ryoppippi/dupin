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

use dupin_core::Batch;
use dupin_core::catalogue::{Catalogue, Change, ChangeError, Entry, Limits};
use dupin_js_runtime::row::{MAX_PAGE_ROWS, PageChange};
use dupin_js_runtime::{CataloguePage, Diagnostic, Reason, Row, RunError};

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
pub(crate) type Fetch<'a> = dyn FnMut(Option<&str>, Reason) -> Result<CataloguePage, RunError> + 'a;

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
}

impl Default for Cached {
    fn default() -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                catalogue: Catalogue::default(),
                cursor: None,
                expires: None,
                due: Some(Reason::Load),
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
    /// missing, invalidated or expired. `convert` turns each row the package sent into an entry.
    pub fn search(
        &self,
        query: &str,
        limit: usize,
        fetch: &mut Fetch<'_>,
        convert: &dyn Fn(Row) -> Entry,
    ) -> Result<Vec<Batch>, RunError> {
        let mut state = self.shared.lock();
        self.shared.used.store(now_ms(), Ordering::Relaxed);
        let refreshed = state.refresh(Instant::now(), fetch, convert);
        self.shared
            .bytes
            .store(state.catalogue.bytes(), Ordering::Relaxed);
        refreshed?;
        let batches = state.catalogue.search(query, limit);
        drop(state);
        self.keep_within_budget();
        Ok(batches)
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

impl State {
    fn forget(&mut self) {
        self.catalogue = Catalogue::default();
        self.cursor = None;
        self.expires = None;
        self.due = Some(Reason::Load);
    }

    fn refresh(
        &mut self,
        now: Instant,
        fetch: &mut Fetch<'_>,
        convert: &dyn Fn(Row) -> Entry,
    ) -> Result<(), RunError> {
        let expired = self.expires.is_some_and(|expires| now >= expires);
        let Some(mut reason) = self.due.or(expired.then_some(Reason::Expired)) else {
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
            let page = fetch(cursor.as_deref(), reason).inspect_err(|_| self.forget())?;
            let change = match page.change {
                PageChange::Reset { rows } => Change::Reset(rows.into_iter().map(convert).collect()),
                PageChange::Update {
                    append,
                    replace,
                    remove,
                } => Change::Update {
                    append: append.into_iter().map(convert).collect(),
                    replace: replace.into_iter().map(convert).collect(),
                    remove,
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
        }
    }

    fn entry(row: Row) -> Entry {
        Entry {
            item: Item {
                id: row.id,
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
        }
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
        let mut fetch = |cursor: Option<&str>, reason| {
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
            cached.search(query, 10, &mut fetch, &entry).unwrap();
        }
        assert_eq!(
            titles(&cached.search("no", 10, &mut fetch, &entry).unwrap()),
            ["Notes"]
        );
        assert_eq!(calls, [(None, Reason::Load)]);
    }

    #[test]
    fn parts_and_updates_follow_the_cursor_and_invalidation_asks_with_it() {
        let cached = Cached::default();
        let mut calls = Vec::new();
        let mut fetch = |cursor: Option<&str>, reason| {
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
            titles(&cached.search("", 10, &mut fetch, &entry).unwrap()),
            ["Alpha", "Beta"]
        );
        cached.invalidate();
        assert_eq!(
            titles(&cached.search("", 10, &mut fetch, &entry).unwrap()),
            ["Bravo"]
        );
        assert_eq!(calls.len(), 3);
    }

    #[test]
    fn an_expired_catalogue_is_asked_again_and_a_failure_starts_over() {
        let cached = Cached::default();
        let mut reasons = Vec::new();
        let mut fetch = |_: Option<&str>, reason| {
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
        cached.search("", 10, &mut fetch, &entry).unwrap();
        cached.shared.lock().expires = Some(Instant::now());
        assert!(cached.search("", 10, &mut fetch, &entry).is_err());
        assert_eq!(cached.len(), 0);
        assert_eq!(titles(&cached.search("", 10, &mut fetch, &entry).unwrap()), ["B"]);
        assert_eq!(reasons, [Reason::Load, Reason::Expired, Reason::Load]);
    }

    #[test]
    fn a_rejected_change_or_endless_parts_drop_the_catalogue() {
        let cached = Cached::default();
        let mut fetch = |_: Option<&str>, _| {
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
        let error = cached.search("", 10, &mut fetch, &entry).unwrap_err();
        assert!(
            matches!(&error, RunError::Failed(diagnostic) if diagnostic.code == "unknown-id"),
            "{error:?}"
        );
        let mut endless = |_: Option<&str>, _| Ok(page(PageChange::Reset { rows: Vec::new() }, "1", true));
        let error = cached.search("", 10, &mut endless, &entry).unwrap_err();
        assert!(
            matches!(&error, RunError::Failed(diagnostic) if diagnostic.code == "too-many-parts"),
            "{error:?}"
        );
    }
}
