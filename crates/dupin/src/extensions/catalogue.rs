//! The `items()` catalogue Dupin holds for each loaded package revision. It is asked for on
//! first use, after an action invalidates it and once its `ttl` passes, never per keystroke:
//! typing matches the rows held here natively. Once only its `ttl` has passed, the launcher asks
//! again in the background while typing keeps matching the rows held, so a slow package never
//! holds up a keystroke for a list it already gave. A package revision Dupin reloads starts a new,
//! empty catalogue with the new [`super::Extension`].
//!
//! A catalogue may read the package's storage, files or the network, as a search may, so it
//! belongs to the broker generation it was asked under: once the package's authority changes,
//! such as a revoked scope, the rows it held are dropped and asked for again from the start.
//!
//! Catalogues live in memory only. Rebuilding one costs a call to the package, and one written
//! to disk would keep what a package lists, such as clipboard history, past the package's own
//! say; a package that wants its list to outlast a restart keeps it in its own storage.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant};

use dupin_core::catalogue::{Catalogue, Change, ChangeError, Entry, Limits, Matching};
use dupin_core::{Batch, Item};
use dupin_extension_bridge::Generation;
use dupin_js_runtime::row::{MAX_PAGE_BYTES, MAX_PAGE_ROWS, PageChange};
use dupin_js_runtime::web_search::WebSearch;
use dupin_js_runtime::{CataloguePage, Diagnostic, Preferences, Reason, Row, RunError};

use super::{Clock, Role};

/// What one package's catalogue may hold.
pub const LIMITS: Limits = Limits {
    rows: 10_000,
    bytes: 8 * 1024 * 1024,
};
/// Text all catalogues together may hold. Past it, the least recently used are dropped and
/// asked for again on their next use.
const BUDGET_BYTES: usize = 24 * 1024 * 1024;
/// Parts one refresh may take: a full catalogue whose parts each end at an answer's rows or at
/// its text, whichever comes first, and one more.
const MAX_PARTS: usize = LIMITS.rows / MAX_PAGE_ROWS + LIMITS.bytes / MAX_PAGE_BYTES + 1;

pub(crate) struct Cached {
    shared: Arc<Shared>,
    clock: Clock,
}

/// Asks the package for the next part of its catalogue, after the cursor it gave.
pub(crate) type Fetch<'a> =
    dyn FnMut(Option<&str>, Reason, &Arc<Preferences>) -> Result<CataloguePage, RunError> + 'a;

/// How the rows a package sends become the entries Dupin holds. An entry's ID need not be the
/// row's, so the IDs an update removes are mapped the same way before they reach the catalogue.
pub(crate) trait Convert {
    fn entry(&self, row: Row) -> Entry;
    /// The ID of the entry [`Convert::entry`] makes from a row whose ID is `id`.
    fn entry_id(&self, id: &str) -> String;
}

/// How a catalogue is asked for: with the preferences the package is handed, under the broker
/// generation its host requests run in, and for which search.
#[derive(Clone, Copy)]
pub(crate) struct Asking<'a> {
    pub preferences: &'a Arc<Preferences>,
    pub generation: Option<Generation>,
    pub role: Role,
}

struct Shared {
    state: Mutex<State>,
    /// Kept outside the lock, so the budget can weigh a catalogue another thread is filling.
    bytes: AtomicUsize,
    used: AtomicU64,
    /// Counts [`Cached::forget`] calls outside the lock, which a catalogue being filled holds
    /// for as long as the package takes, so forgetting never waits for it.
    forgotten: AtomicU64,
}

struct State {
    catalogue: Catalogue,
    cursor: Option<String>,
    expires: Option<Instant>,
    /// Why the package must be asked before the catalogue is used again, if it must.
    due: Option<Reason>,
    /// The preferences the catalogue was asked for with.
    preferences: Option<Arc<Preferences>>,
    /// The broker generation the rows were asked under.
    generation: Option<Generation>,
    /// The [`Shared::forgotten`] count the rows were asked after.
    forgotten: u64,
    /// The preferences the last ask failed with, while nothing else has changed since that
    /// could make the next one succeed.
    failed: Option<Arc<Preferences>>,
    /// Whether the package is being asked again in the background for a catalogue that has
    /// only outlived its `ttl`, which searches meanwhile answer from.
    refreshing: bool,
}

impl Default for Cached {
    fn default() -> Self {
        Self::new(Matching::default())
    }
}

impl Cached {
    pub fn new(matching: Matching) -> Self {
        Self::with_clock(matching, Clock::launcher())
    }

    /// One whose rows' times are measured against `clock`.
    pub fn with_clock(matching: Matching, clock: Clock) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                catalogue: Catalogue::new(matching),
                cursor: None,
                expires: None,
                due: Some(Reason::Load),
                preferences: None,
                generation: None,
                forgotten: 0,
                failed: None,
                refreshing: false,
            }),
            bytes: AtomicUsize::new(0),
            used: AtomicU64::new(0),
            forgotten: AtomicU64::new(0),
        });
        held()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(Arc::downgrade(&shared));
        Self { shared, clock }
    }

    /// Drops what the catalogue holds when it is next used, which then asks for it from the
    /// start. Rows being asked for right now are dropped at that use too.
    pub fn forget(&self) {
        self.shared.forgotten.fetch_add(1, Ordering::AcqRel);
    }

    /// Asks the package again before the next use, with the cursor it last gave.
    pub fn invalidate(&self) {
        let mut state = self.shared.lock();
        state.failed = None;
        if state.due.is_none() {
            state.due = Some(Reason::Invalidated);
        }
    }

    /// The rows for `query`, after asking the package through `fetch` if the catalogue is
    /// missing, invalidated, expired, forgotten, asked for with other preferences or held from
    /// another generation than the one `fetch` asks under. The main search doesn't ask again
    /// after a failure until one of those changes or the keyword's search succeeds, so a
    /// package that can't answer it, such as for want of consent, costs no call per keystroke.
    /// Web searches are reached by their alias instead, through [`Cached::web_searches`].
    pub fn search(
        &self,
        query: &str,
        limit: usize,
        asking: Asking<'_>,
        fetch: &mut Fetch<'_>,
        convert: &dyn Convert,
    ) -> Result<Vec<Batch>, RunError> {
        self.fresh(asking, fetch, convert, |catalogue| {
            catalogue.search(query, limit, self.clock.now_ms() / 1000)
        })
    }

    /// The catalogue's web searches, each with the row it shows before a query is typed,
    /// refreshed as [`Cached::search`] refreshes the catalogue.
    pub fn web_searches(
        &self,
        asking: Asking<'_>,
        fetch: &mut Fetch<'_>,
        convert: &dyn Convert,
    ) -> Result<Vec<(Item, WebSearch)>, RunError> {
        self.fresh(asking, fetch, convert, held_web_searches)
    }

    /// The web searches [`Cached::web_searches`] would return for `preferences` without asking
    /// the package, if Dupin holds them and can read them without waiting for a search that is
    /// filling the catalogue.
    pub fn held_web_searches(&self, preferences: &Arc<Preferences>) -> Option<Vec<(Item, WebSearch)>> {
        let state = self.shared.state.try_lock().ok()?;
        let current = state.due.is_none()
            && state.forgotten == self.shared.forgotten.load(Ordering::Acquire)
            && !state.expires.is_some_and(|expires| Instant::now() >= expires)
            && state
                .preferences
                .as_ref()
                .is_some_and(|held| Arc::ptr_eq(held, preferences));
        current.then(|| held_web_searches(&state.catalogue))
    }

    /// When the catalogue held has only outlived its `ttl`, so it can still answer while it
    /// is asked for again: whether the caller is the one to ask, with
    /// [`Cached::refresh_stale`], which it then must. Searches answer from the rows held until
    /// that ends.
    pub fn stale(&self, asking: Asking<'_>) -> Option<bool> {
        let mut state = self.shared.lock();
        let current = state.generation == asking.generation
            && state.forgotten == self.shared.forgotten.load(Ordering::Acquire)
            && state.due.is_none()
            && state.failed.is_none()
            && state
                .preferences
                .as_ref()
                .is_some_and(|held| Arc::ptr_eq(held, asking.preferences))
            && state.expires.is_some_and(|expires| Instant::now() >= expires);
        current.then(|| !std::mem::replace(&mut state.refreshing, true))
    }

    /// Asks the package again for the catalogue [`Cached::stale`] said is stale, without
    /// holding it while the package answers, and applies the answer only if nothing changed the
    /// catalogue meanwhile, such as an invalidation, new preferences or lost authority, which
    /// ask again on their own. A failure drops the catalogue, as it does for a search. Reports
    /// whether the catalogue changed, so what lists it can search again.
    pub fn refresh_stale(
        &self,
        preferences: &Arc<Preferences>,
        generation: Option<Generation>,
        fetch: &mut Fetch<'_>,
        convert: &dyn Convert,
    ) -> bool {
        struct Release<'a>(&'a Shared);
        impl Drop for Release<'_> {
            fn drop(&mut self) {
                self.0.lock().refreshing = false;
            }
        }
        let release = Release(&self.shared);
        let (mut cursor, forgotten) = {
            let state = self.shared.lock();
            (state.cursor.clone(), state.forgotten)
        };
        let now = Instant::now();
        let mut reason = Reason::Expired;
        let mut pages = Vec::new();
        let mut fetched = Err(RunError::Failed(too_many_parts()));
        for _ in 0..MAX_PARTS {
            match fetch(cursor.as_deref(), reason, preferences) {
                Ok(page) => {
                    cursor.clone_from(&page.cursor);
                    let more = page.more;
                    pages.push(page);
                    if !more {
                        fetched = Ok(());
                        break;
                    }
                    reason = Reason::More;
                }
                Err(error) => {
                    fetched = Err(error);
                    break;
                }
            }
        }
        let mut state = self.shared.lock();
        let unchanged = state.generation == generation
            && state.forgotten == forgotten
            && self.shared.forgotten.load(Ordering::Acquire) == forgotten
            && state.due.is_none()
            && state
                .preferences
                .as_ref()
                .is_some_and(|held| Arc::ptr_eq(held, preferences));
        if !unchanged {
            return false;
        }
        let applied = fetched.and_then(|()| {
            pages
                .into_iter()
                .try_for_each(|page| state.apply(page, now, preferences, convert).map(drop))
        });
        if applied.is_err() {
            state.forget();
            state.failed = Some(preferences.clone());
        }
        self.shared
            .bytes
            .store(state.catalogue.bytes(), Ordering::Relaxed);
        drop(state);
        drop(release);
        self.keep_within_budget();
        true
    }

    /// When the rows held outlive their `ttl`, if they do and Dupin holds rows.
    pub fn expires(&self) -> Option<Instant> {
        let state = self.shared.lock();
        state.expires.filter(|_| state.due.is_none())
    }

    fn fresh<T: Default>(
        &self,
        asking: Asking<'_>,
        fetch: &mut Fetch<'_>,
        convert: &dyn Convert,
        read: impl FnOnce(&Catalogue) -> T,
    ) -> Result<T, RunError> {
        let mut state = self.shared.lock();
        self.shared.used.store(now_ms(), Ordering::Relaxed);
        let forgotten = self.shared.forgotten.load(Ordering::Acquire);
        if state.generation != asking.generation || state.forgotten != forgotten {
            state.forget();
            state.generation = asking.generation;
            state.forgotten = forgotten;
        }
        let held_off = state
            .failed
            .as_ref()
            .is_some_and(|failed| Arc::ptr_eq(failed, asking.preferences));
        if held_off && asking.role == Role::DefaultSearch {
            self.shared.bytes.store(0, Ordering::Relaxed);
            return Ok(T::default());
        }
        let refreshed = state.refresh(Instant::now(), asking.preferences, fetch, convert);
        state.failed = refreshed.is_err().then(|| asking.preferences.clone());
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
        let listed = self.shared.lock().catalogue.search("", usize::MAX, 0);
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
        self.catalogue = Catalogue::new(self.catalogue.matching());
        self.cursor = None;
        self.expires = None;
        self.due = Some(Reason::Load);
        self.preferences = None;
        self.failed = None;
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
        if reason == Reason::Expired && self.refreshing {
            return Ok(());
        }
        for _ in 0..MAX_PARTS {
            let cursor = if reason == Reason::Load {
                None
            } else {
                self.cursor.clone()
            };
            let page = fetch(cursor.as_deref(), reason, preferences).inspect_err(|_| self.forget())?;
            if self.apply(page, now, preferences, convert)? {
                return Ok(());
            }
            reason = Reason::More;
        }
        self.forget();
        Err(RunError::Failed(too_many_parts()))
    }

    /// Applies one part of an answer. Whether it was the last part.
    fn apply(
        &mut self,
        page: CataloguePage,
        now: Instant,
        preferences: &Arc<Preferences>,
        convert: &dyn Convert,
    ) -> Result<bool, RunError> {
        let change = match page.change {
            PageChange::Reset { rows } => {
                Change::Reset(rows.into_iter().map(|row| convert.entry(row)).collect())
            }
            PageChange::Update {
                prepend,
                append,
                replace,
                remove,
            } => Change::Update {
                prepend: prepend.into_iter().map(|row| convert.entry(row)).collect(),
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
        if page.more {
            return Ok(false);
        }
        self.due = None;
        self.preferences = Some(preferences.clone());
        Ok(true)
    }
}

fn too_many_parts() -> Diagnostic {
    Diagnostic::new(
        "too-many-parts",
        format!("items kept answering `more: true` after {MAX_PARTS} parts"),
    )
    .hint("Set `more` only while rows are left to send.")
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
        Row::new(id, title)
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
                aliases: row.aliases,
                keywords: row.keywords,
                section: None,
                detail: None,
                web_search: row.web_search,
                match_text: None,
                time: row.time.map(|ms| ms / 1000),
                file: None,
                file_actions: Vec::new(),
                accessories: Vec::new(),
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

    fn keyword(preferences: &Arc<Preferences>) -> Asking<'_> {
        Asking {
            preferences,
            generation: None,
            role: Role::Keyword,
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

    fn subtitles(batches: &[Batch]) -> Vec<String> {
        batches
            .iter()
            .flat_map(|batch| &batch.items)
            .map(|item| match &item.view {
                View::Text { subtitle, .. } => subtitle.clone(),
                _ => String::new(),
            })
            .collect()
    }

    #[test]
    fn a_fixed_clock_says_how_long_ago_rows_are_whatever_the_wall_clock_says() {
        const NOW: i64 = 1_795_942_800_000;
        let shown = |wall: fn() -> u64| {
            let cached = Cached::with_clock(
                Matching::default(),
                Clock {
                    fixed: Some(NOW),
                    wall,
                },
            );
            let mut fetch = |_: Option<&str>, _, _: &Arc<Preferences>| {
                let mut copied = row("a", "Copied");
                copied.time = Some(NOW as u64 - 7_200_000);
                Ok(page(PageChange::Reset { rows: vec![copied] }, "1", false))
            };
            subtitles(
                &cached
                    .search("", 10, keyword(&none()), &mut fetch, ENTRY)
                    .unwrap(),
            )
        };
        let today = shown(|| 1_795_942_800_000);
        assert_eq!(today, ["2 h ago"]);
        assert_eq!(shown(|| 1_796_202_000_000), today, "three days later");
    }

    #[test]
    fn the_package_is_asked_once_and_typing_only_matches_what_is_held() {
        let cached = Cached::default();
        let mut calls = Vec::new();
        let mut fetch = |cursor: Option<&str>, reason, _: &Arc<Preferences>| {
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
            cached
                .search(query, 10, keyword(&none()), &mut fetch, ENTRY)
                .unwrap();
        }
        assert_eq!(
            titles(
                &cached
                    .search("no", 10, keyword(&none()), &mut fetch, ENTRY)
                    .unwrap()
            ),
            ["Notes"]
        );
        assert_eq!(calls, [(None, Reason::Load)]);
    }

    #[test]
    fn parts_and_updates_follow_the_cursor_and_invalidation_asks_with_it() {
        let cached = Cached::default();
        let mut calls = Vec::new();
        let mut fetch = |cursor: Option<&str>, reason, _: &Arc<Preferences>| {
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
                        prepend: Vec::new(),
                        append: vec![row("b", "Beta")],
                        replace: Vec::new(),
                        remove: Vec::new(),
                    },
                    "2",
                    false,
                ),
                (Some("2"), Reason::Invalidated) => page(
                    PageChange::Update {
                        prepend: Vec::new(),
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
            titles(
                &cached
                    .search("", 10, keyword(&none()), &mut fetch, ENTRY)
                    .unwrap()
            ),
            ["Alpha", "Beta"]
        );
        cached.invalidate();
        assert_eq!(
            titles(
                &cached
                    .search("", 10, keyword(&none()), &mut fetch, ENTRY)
                    .unwrap()
            ),
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
                prepend: Vec::new(),
                append: Vec::new(),
                replace: Vec::new(),
                remove: vec!["clip-1".into()],
            },
            PageChange::Update {
                prepend: Vec::new(),
                append: vec![row("clip-1", "Again")],
                replace: Vec::new(),
                remove: Vec::new(),
            },
        ]
        .into_iter();
        let mut fetch = |_: Option<&str>, _, _: &Arc<Preferences>| {
            Ok(page(answers.next().expect("asked once per change"), "1", false))
        };
        assert_eq!(
            titles(
                &cached
                    .search("", 10, keyword(&none()), &mut fetch, ENTRY)
                    .unwrap()
            ),
            ["First", "Second"]
        );
        cached.invalidate();
        assert_eq!(
            titles(
                &cached
                    .search("", 10, keyword(&none()), &mut fetch, ENTRY)
                    .unwrap()
            ),
            ["Second"]
        );
        cached.invalidate();
        assert_eq!(
            titles(
                &cached
                    .search("", 10, keyword(&none()), &mut fetch, ENTRY)
                    .unwrap()
            ),
            ["Second", "Again"]
        );
    }

    #[test]
    fn a_stale_catalogue_answers_from_what_it_holds_while_one_caller_asks_again() {
        let cached = Cached::default();
        let preferences = none();
        let answer = |title: &str| {
            Ok(CataloguePage {
                ttl: Some(60),
                ..page(
                    PageChange::Reset {
                        rows: vec![row("a", title)],
                    },
                    "1",
                    false,
                )
            })
        };
        let mut never = |_: Option<&str>, _, _: &Arc<Preferences>| -> Result<CataloguePage, RunError> {
            panic!("a stale catalogue answers without asking")
        };
        cached
            .search("", 10, keyword(&preferences), &mut |_, _, _| answer("Old"), ENTRY)
            .unwrap();
        assert_eq!(cached.stale(keyword(&preferences)), None, "it is fresh");
        cached.shared.lock().expires = Some(Instant::now());
        assert_eq!(cached.stale(keyword(&preferences)), Some(true));
        assert_eq!(
            cached.stale(keyword(&preferences)),
            Some(false),
            "only one caller asks again"
        );
        assert_eq!(
            titles(
                &cached
                    .search("", 10, keyword(&preferences), &mut never, ENTRY)
                    .unwrap()
            ),
            ["Old"]
        );
        let mut reasons = Vec::new();
        cached.refresh_stale(
            &preferences,
            None,
            &mut |_, reason, _| {
                reasons.push(reason);
                answer("New")
            },
            ENTRY,
        );
        assert_eq!(reasons, [Reason::Expired]);
        assert_eq!(cached.stale(keyword(&preferences)), None);
        assert_eq!(
            titles(
                &cached
                    .search("", 10, keyword(&preferences), &mut never, ENTRY)
                    .unwrap()
            ),
            ["New"]
        );

        // An invalidation while the package answers asks again on its own; the stale answer is
        // dropped rather than applied over it.
        cached.shared.lock().expires = Some(Instant::now());
        assert_eq!(cached.stale(keyword(&preferences)), Some(true));
        cached.refresh_stale(
            &preferences,
            None,
            &mut |_, _, _| {
                cached.invalidate();
                answer("Stale")
            },
            ENTRY,
        );
        let mut asked = Vec::new();
        assert_eq!(
            titles(
                &cached
                    .search(
                        "",
                        10,
                        keyword(&preferences),
                        &mut |_, reason, _| {
                            asked.push(reason);
                            answer("Invalidated")
                        },
                        ENTRY
                    )
                    .unwrap()
            ),
            ["Invalidated"]
        );
        assert_eq!(asked, [Reason::Invalidated]);

        // A failure drops the catalogue, as it does for a search.
        cached.shared.lock().expires = Some(Instant::now());
        assert_eq!(cached.stale(keyword(&preferences)), Some(true));
        cached.refresh_stale(
            &preferences,
            None,
            &mut |_, _, _| Err(RunError::Failed(Diagnostic::new("items-threw", "boom"))),
            ENTRY,
        );
        assert_eq!(cached.len(), 0);
        assert_eq!(cached.stale(keyword(&preferences)), None);
    }

    #[test]
    fn an_expired_catalogue_is_asked_again_and_a_failure_starts_over() {
        let cached = Cached::default();
        let mut reasons = Vec::new();
        let mut fetch = |_: Option<&str>, reason, _: &Arc<Preferences>| {
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
        cached
            .search("", 10, keyword(&none()), &mut fetch, ENTRY)
            .unwrap();
        cached.shared.lock().expires = Some(Instant::now());
        assert!(
            cached
                .search("", 10, keyword(&none()), &mut fetch, ENTRY)
                .is_err()
        );
        assert_eq!(cached.len(), 0);
        assert_eq!(
            titles(
                &cached
                    .search("", 10, keyword(&none()), &mut fetch, ENTRY)
                    .unwrap()
            ),
            ["B"]
        );
        assert_eq!(reasons, [Reason::Load, Reason::Expired, Reason::Load]);
    }

    #[test]
    fn the_main_search_asks_a_failed_catalogue_again_only_once_something_changed() {
        use std::cell::{Cell, RefCell};

        let cached = Cached::default();
        let calls = RefCell::new(Vec::new());
        let failing = Cell::new(true);
        let ask = |role| {
            let mut fetch = |_: Option<&str>, reason, _: &Arc<Preferences>| {
                calls.borrow_mut().push(reason);
                if failing.get() {
                    Err(RunError::Failed(Diagnostic::new("items-threw", "needs-consent")))
                } else {
                    Ok(page(
                        PageChange::Reset {
                            rows: vec![row("a", "Allowed")],
                        },
                        "1",
                        false,
                    ))
                }
            };
            cached.search(
                "a",
                10,
                Asking {
                    preferences: &none(),
                    generation: None,
                    role,
                },
                &mut fetch,
                ENTRY,
            )
        };
        assert!(ask(Role::DefaultSearch).is_err());
        for _ in 0..3 {
            assert!(
                ask(Role::DefaultSearch).unwrap().is_empty(),
                "held off, not failing again"
            );
        }
        assert_eq!(calls.borrow().len(), 1, "one call, not one per keystroke");

        assert!(
            ask(Role::Keyword).is_err(),
            "the keyword still asks, so it can show the failure and any question"
        );
        assert_eq!(calls.borrow().len(), 2);

        cached.forget();
        assert!(
            ask(Role::DefaultSearch).is_err(),
            "forgetting, as an access change does, asks again"
        );
        assert_eq!(calls.borrow().len(), 3);
        cached.invalidate();
        failing.set(false);
        assert_eq!(titles(&ask(Role::DefaultSearch).unwrap()), ["Allowed"]);
        assert_eq!(
            *calls.borrow(),
            [Reason::Load, Reason::Load, Reason::Load, Reason::Load]
        );
    }

    #[test]
    fn other_preferences_ask_a_failed_catalogue_again_for_web_searches() {
        let cached = Cached::default();
        let mut calls = 0;
        let mut fetch = |_: Option<&str>, _, _: &Arc<Preferences>| {
            calls += 1;
            Err(RunError::Failed(Diagnostic::new(
                "items-threw",
                "bad preferences",
            )))
        };
        let main = |preferences| Asking {
            preferences,
            generation: None,
            role: Role::DefaultSearch,
        };
        let first = none();
        assert!(cached.web_searches(main(&first), &mut fetch, ENTRY).is_err());
        assert!(
            cached
                .web_searches(main(&first), &mut fetch, ENTRY)
                .unwrap()
                .is_empty(),
            "held off with the same preferences"
        );
        let changed = Arc::new(Preferences::from_iter([(
            "webSearches".into(),
            serde_json::json!({}),
        )]));
        assert!(cached.web_searches(main(&changed), &mut fetch, ENTRY).is_err());
        assert_eq!(calls, 2);
    }

    #[test]
    fn forgetting_never_waits_for_a_catalogue_being_asked_for() {
        use std::sync::mpsc;

        let cached = Arc::new(Cached::default());
        let (asked, was_asked) = mpsc::channel();
        let (answer, answered) = mpsc::channel::<()>();
        let filling = std::thread::spawn({
            let cached = cached.clone();
            move || {
                let mut fetch = |_: Option<&str>, _, _: &Arc<Preferences>| {
                    asked.send(()).unwrap();
                    answered.recv().unwrap();
                    Ok(page(
                        PageChange::Reset {
                            rows: vec![row("a", "Before")],
                        },
                        "1",
                        false,
                    ))
                };
                titles(
                    &cached
                        .search("", 10, keyword(&none()), &mut fetch, ENTRY)
                        .unwrap(),
                )
            }
        });
        was_asked.recv().unwrap();
        cached.forget();
        answer.send(()).unwrap();
        assert_eq!(filling.join().unwrap(), ["Before"]);

        let mut reasons = Vec::new();
        let mut fetch = |_: Option<&str>, reason, _: &Arc<Preferences>| {
            reasons.push(reason);
            Ok(page(
                PageChange::Reset {
                    rows: vec![row("b", "After")],
                },
                "2",
                false,
            ))
        };
        assert_eq!(
            titles(
                &cached
                    .search("", 10, keyword(&none()), &mut fetch, ENTRY)
                    .unwrap()
            ),
            ["After"],
            "rows asked for before the forget are dropped at the next use"
        );
        assert_eq!(reasons, [Reason::Load]);
    }

    #[test]
    fn a_catalogue_from_another_generation_is_asked_for_again_from_the_start() {
        use dupin_extension_bridge::{
            Base, Bridge, Broker, Capabilities, Local, PackageId, Revision, Services,
        };

        let bridge = Bridge::new(
            Broker::new(Base::default(), Local::default()),
            None,
            Services::none(),
        );
        let id = PackageId::new("clips").unwrap();
        let capabilities =
            Capabilities::parse(&serde_json::json!({ "storage": { "quotaBytes": 1024 } })).unwrap();
        bridge.install(&id, Revision::new("blake3:1").unwrap(), capabilities);
        bridge.set_enabled(&id, true).unwrap();
        let before = bridge
            .call(&id, dupin_extension_bridge::Trigger::Query)
            .generation;
        bridge.set_enabled(&id, false).unwrap();
        let after = bridge
            .call(&id, dupin_extension_bridge::Trigger::Query)
            .generation;
        assert_ne!(before, after);

        let cached = Cached::default();
        let mut calls = Vec::new();
        let mut fetch = |cursor: Option<&str>, reason, _: &Arc<Preferences>| {
            calls.push((cursor.map(str::to_owned), reason));
            Ok(page(
                PageChange::Reset {
                    rows: vec![row("a", "Copied")],
                },
                "1",
                false,
            ))
        };
        cached
            .search(
                "",
                10,
                Asking {
                    preferences: &none(),
                    generation: before,
                    role: Role::Keyword,
                },
                &mut fetch,
                ENTRY,
            )
            .unwrap();
        cached
            .search(
                "c",
                10,
                Asking {
                    preferences: &none(),
                    generation: before,
                    role: Role::Keyword,
                },
                &mut fetch,
                ENTRY,
            )
            .unwrap();
        cached.invalidate();
        cached
            .search(
                "",
                10,
                Asking {
                    preferences: &none(),
                    generation: after,
                    role: Role::Keyword,
                },
                &mut fetch,
                ENTRY,
            )
            .unwrap();
        assert_eq!(
            calls,
            [(None, Reason::Load), (None, Reason::Load)],
            "a new generation starts over instead of updating rows asked for under the old one"
        );
    }

    #[test]
    fn a_catalogue_near_both_limits_arrives_in_the_parts_its_rows_and_text_need() {
        let cached = Cached::default();
        let heavy = 7;
        let full = 19;
        let mut sent = 0;
        let mut fetch = |_: Option<&str>, _, _: &Arc<Preferences>| {
            sent += 1;
            let rows = if sent <= heavy {
                let mut large = row(&format!("heavy-{sent}"), "Heavy");
                large.subtitle = "x".repeat(MAX_PAGE_BYTES - 64);
                vec![large]
            } else {
                (0..MAX_PAGE_ROWS)
                    .map(|index| row(&format!("{sent}-{index}"), "Light"))
                    .collect()
            };
            let change = if sent == 1 {
                PageChange::Reset { rows }
            } else {
                PageChange::Update {
                    prepend: Vec::new(),
                    append: rows,
                    replace: Vec::new(),
                    remove: Vec::new(),
                }
            };
            Ok(page(change, "1", sent < heavy + full))
        };
        assert!(heavy + full > LIMITS.rows / MAX_PAGE_ROWS + 1);
        let found = cached
            .search("heavy", 10, keyword(&none()), &mut fetch, ENTRY)
            .unwrap();
        assert_eq!(titles(&found).len(), heavy);
    }

    #[test]
    fn a_rejected_change_or_endless_parts_drop_the_catalogue() {
        let cached = Cached::default();
        let mut fetch = |_: Option<&str>, _, _: &Arc<Preferences>| {
            Ok(page(
                PageChange::Update {
                    prepend: Vec::new(),
                    append: Vec::new(),
                    replace: vec![row("missing", "M")],
                    remove: Vec::new(),
                },
                "1",
                false,
            ))
        };
        let error = cached
            .search("", 10, keyword(&none()), &mut fetch, ENTRY)
            .unwrap_err();
        assert!(
            matches!(&error, RunError::Failed(diagnostic) if diagnostic.code == "unknown-id"),
            "{error:?}"
        );
        let mut endless = |_: Option<&str>, _, _: &Arc<Preferences>| {
            Ok(page(PageChange::Reset { rows: Vec::new() }, "1", true))
        };
        let error = cached
            .search("", 10, keyword(&none()), &mut endless, ENTRY)
            .unwrap_err();
        assert!(
            matches!(&error, RunError::Failed(diagnostic) if diagnostic.code == "too-many-parts"),
            "{error:?}"
        );
    }

    #[test]
    fn other_preferences_load_the_catalogue_again() {
        let cached = Cached::default();
        let mut seen = Vec::new();
        let mut fetch = |_: Option<&str>, reason, preferences: &Arc<Preferences>| {
            seen.push((reason, preferences.len()));
            Ok(page(
                PageChange::Reset {
                    rows: vec![row("a", "Alpha")],
                },
                "1",
                false,
            ))
        };
        let first = none();
        cached.search("", 10, keyword(&first), &mut fetch, ENTRY).unwrap();
        cached
            .search("a", 10, keyword(&first), &mut fetch, ENTRY)
            .unwrap();
        let changed = Arc::new(Preferences::from_iter([("paste".into(), true.into())]));
        cached
            .search("a", 10, keyword(&changed), &mut fetch, ENTRY)
            .unwrap();
        cached
            .search("al", 10, keyword(&changed), &mut fetch, ENTRY)
            .unwrap();
        assert_eq!(seen, [(Reason::Load, 0), (Reason::Load, 1)]);
    }

    #[test]
    fn other_preferences_ask_a_failed_catalogue_again_from_the_main_search() {
        let cached = Cached::default();
        let mut calls = 0;
        let mut fetch = |_: Option<&str>, _, _: &Arc<Preferences>| {
            calls += 1;
            Err(RunError::Failed(Diagnostic::new(
                "items-threw",
                "bad preferences",
            )))
        };
        let main = |preferences| Asking {
            preferences,
            generation: None,
            role: Role::DefaultSearch,
        };
        let first = none();
        assert!(cached.search("a", 10, main(&first), &mut fetch, ENTRY).is_err());
        assert!(
            cached
                .search("a", 10, main(&first), &mut fetch, ENTRY)
                .unwrap()
                .is_empty(),
            "held off with the same preferences"
        );
        let changed = Arc::new(Preferences::from_iter([("paste".into(), true.into())]));
        assert!(cached.search("a", 10, main(&changed), &mut fetch, ENTRY).is_err());
        assert_eq!(calls, 2);
    }

    #[test]
    fn web_searches_come_from_the_catalogue_held_for_the_current_preferences() {
        use dupin_js_runtime::web_search::WebSearch;

        let cached = Cached::default();
        let mut seen = Vec::new();
        let mut fetch = |_: Option<&str>, reason, preferences: &Arc<Preferences>| {
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
        let first = none();
        assert_eq!(cached.held_web_searches(&first), None);
        let searches = cached.web_searches(keyword(&first), &mut fetch, ENTRY).unwrap();
        assert_eq!(searches.len(), 1);
        assert_eq!(searches[0].0.id, "held/g");
        assert_eq!(searches[0].1.alias, "g");
        assert_eq!(cached.held_web_searches(&first).unwrap().len(), 1);
        assert_eq!(
            titles(&cached.search("", 10, keyword(&first), &mut fetch, ENTRY).unwrap()),
            ["Alpha"]
        );
        let changed = Arc::new(Preferences::from_iter([(
            "webFallbacks".into(),
            serde_json::json!(["g"]),
        )]));
        assert_eq!(
            cached.held_web_searches(&changed),
            None,
            "held for other preferences"
        );
        cached.web_searches(keyword(&changed), &mut fetch, ENTRY).unwrap();
        assert_eq!(seen, [(Reason::Load, 0), (Reason::Load, 1)]);
        assert_eq!(cached.held_web_searches(&changed).unwrap().len(), 1);
        assert_eq!(cached.held_web_searches(&first), None);

        cached.shared.lock().expires = Some(Instant::now());
        assert_eq!(cached.held_web_searches(&changed), None, "expired");
        cached.shared.lock().expires = None;
        let filling = cached.shared.lock();
        assert_eq!(cached.held_web_searches(&changed), None, "being filled");
        drop(filling);
        cached.invalidate();
        assert_eq!(cached.held_web_searches(&changed), None, "invalidated");
    }
}
