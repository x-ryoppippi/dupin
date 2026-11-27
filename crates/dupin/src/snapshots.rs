mod store;

use dupin_core::snapshots::Snapshot;
use rune::{AppHandle, El, Key, KeyEvent, ScrollSpec, TextEdit, Tx, col, el, row, text, text_input};
use rune_kit::Kit;
use store::Store;

use crate::prefs::Prefs;

/// How wide the list of saved snapshots is, beside the selected one.
const LIST_WIDTH: f32 = 260.0;

/// The saved snapshots, as Settings' Snapshots page lists them, and the writer that saves and
/// deletes them.
pub struct Snapshots {
    loaded: bool,
    /// The commands a snapshot can be opened in the panel with, once the panel has said.
    sources: Option<Vec<String>>,
    entries: Vec<Snapshot>,
    filtered: Vec<usize>,
    search: Vec<String>,
    writer: std::sync::mpsc::Sender<Request>,
    archive_problem: Option<String>,
    query: TextEdit,
    selected: Option<String>,
    pending_delete: Option<String>,
    /// The snapshot whose content Copy last put on the clipboard.
    copied: Option<String>,
    problem: Option<String>,
    panel: AppHandle<crate::ui::Ui>,
    running: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Snapshots {
    pub fn new(
        panel: AppHandle<crate::ui::Ui>,
        prefs: AppHandle<Prefs>,
        running: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) -> Self {
        let (writer, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("dupin-snapshots".into())
            .spawn(move || {
                let mut cache = std::collections::HashMap::new();
                write_requests(store::shared(), rx, |store, result| {
                    publish(store, &prefs, result, &mut cache)
                });
            })
            .expect("spawn snapshot writer");
        let mut query = TextEdit::new("");
        query.focused = true;
        Self {
            loaded: false,
            sources: None,
            entries: Vec::new(),
            filtered: Vec::new(),
            search: Vec::new(),
            writer,
            archive_problem: None,
            query,
            selected: None,
            pending_delete: None,
            copied: None,
            problem: None,
            panel,
            running,
        }
    }

    pub fn set_sources(&mut self, sources: Vec<String>) {
        listing().sources.clone_from(&sources);
        self.sources = Some(sources);
    }

    /// Whether the command `mode` is still there to open a snapshot it saved, as far as is known.
    fn available(&self, mode: &str) -> bool {
        self.sources
            .as_ref()
            .is_none_or(|sources| sources.iter().any(|source| source == mode))
    }

    pub fn control(&self, call: crate::control::Call) {
        use crate::control::Command;
        if call.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let result = match &call.request.command {
            Command::SnapshotsList {} if !self.loaded => Err("Busy: snapshots loading".into()),
            Command::SnapshotsList {} => serde_json::to_value(summaries(
                &self.entries,
                self.archive_problem.clone(),
                crate::control::MAX_FRAME / 2,
            ))
            .map_err(|_| "could not encode snapshots".into()),
            Command::SnapshotsRead { .. } if !self.loaded => Err("Busy: snapshots loading".into()),
            Command::SnapshotsRead { id } => match self.entries.iter().find(|s| &s.id == id) {
                Some(entry) => Ok(content(entry, crate::control::MAX_FRAME - 1024)),
                None => Err("NotFound: snapshot".into()),
            },
            Command::SnapshotsOpen { id } => {
                if id.is_some() && !self.loaded {
                    Err("Busy: snapshots loading".into())
                } else if id
                    .as_ref()
                    .is_some_and(|id| !self.entries.iter().any(|s| &s.id == id))
                {
                    Err("NotFound: snapshot".into())
                } else {
                    self.panel
                        .update(move |ui, tx| ui.control_snapshots_open(call, tx));
                    return;
                }
            }
            _ => Err("invalid snapshots command".into()),
        };
        call.finish(result);
    }

    pub fn accept_open(&mut self, call: crate::control::Call, tx: &mut Tx) {
        let id = match &call.request.command {
            crate::control::Command::SnapshotsOpen { id } => id.as_deref(),
            _ => None,
        };
        if id.is_some_and(|id| !self.entries.iter().any(|s| s.id == id)) {
            call.finish(Err("NotFound: snapshot".into()));
            return;
        }
        self.reveal(id, tx);
        call.finish(Ok(serde_json::Value::Null));
    }

    /// Clears the search, as opening the page does, and selects the snapshot `id`, if any.
    pub fn reveal(&mut self, id: Option<&str>, tx: &mut Tx) {
        self.query = TextEdit::new("");
        self.query.focused = true;
        self.pending_delete = None;
        if let Some(id) = id {
            self.open(id);
        }
        self.filter(tx);
    }

    pub fn cancel_delete(&mut self) {
        self.pending_delete = None;
    }

    /// Narrow the list to the query, keeping the selection when it still matches, and scroll to
    /// the selected snapshot: the list would otherwise keep the offset it had before.
    fn filter(&mut self, tx: &mut Tx) {
        let query = dupin_core::snapshots::normalize(&self.query.text);
        self.filtered = self
            .search
            .iter()
            .enumerate()
            .filter_map(|(i, text)| text.contains(&query).then_some(i))
            .collect();
        if !self
            .filtered
            .iter()
            .any(|&i| self.selected.as_ref() == Some(&self.entries[i].id))
        {
            self.selected = self.filtered.first().map(|&i| self.entries[i].id.clone());
        }
        if let Some(id) = &self.selected {
            tx.scroll_into_view(format!("snapshot-{id}"));
        }
    }

    pub fn save(&mut self, snapshot: Snapshot, busy: crate::ui::Busy) {
        if self.writer.send(Request::Save(snapshot, busy)).is_err() {
            self.problem = Some("Snapshot writer is unavailable.".into());
            self.panel
                .update(|ui, tx| ui.snapshot_notice("Snapshot writer is unavailable.".into(), tx));
        }
    }

    pub fn report(&mut self, problem: String) {
        self.problem = Some(problem);
    }

    pub fn open(&mut self, id: &str) -> bool {
        if !self.entries.iter().any(|s| s.id == id) {
            return false;
        }
        self.selected = Some(id.into());
        self.pending_delete = None;
        true
    }

    pub fn delete(&mut self, id: &str) {
        self.pending_delete = None;
        if let Some(index) = self.filtered.iter().position(|&i| self.entries[i].id == id) {
            self.selected = self
                .filtered
                .get(index + 1)
                .or_else(|| index.checked_sub(1).and_then(|i| self.filtered.get(i)))
                .map(|&i| self.entries[i].id.clone());
        }
        let busy = crate::ui::Busy::new(self.running.clone());
        let _ = self.writer.send(Request::Delete(id.into(), busy));
    }

    fn resume(&mut self) {
        if let Some(id) = self.selected.clone() {
            self.resume_id(&id);
        }
    }

    /// Runs the search the snapshot `id` saved again, if its command is still available.
    pub fn resume_id(&mut self, id: &str) {
        if let Some(saved) = self.entries.iter().find(|s| s.id == id).cloned() {
            if !self.available(&saved.mode) {
                self.report("The source command is unavailable. Saved content remains readable.".into());
                return;
            }
            self.panel.update(move |ui, tx| ui.resume_snapshot(saved, tx));
        }
    }

    /// Puts the selected snapshot's saved content on the clipboard, as plain text.
    fn copy(&mut self, tx: &mut Tx) {
        let Some(saved) = self
            .selected
            .as_deref()
            .and_then(|id| self.entries.iter().find(|s| s.id == id))
        else {
            return;
        };
        tx.copy(plain_text(&saved.views));
        self.copied = Some(saved.id.clone());
    }

    /// The page's keys; it leaves the others, ⌘W among them, to the window.
    pub fn key(&mut self, ev: &KeyEvent, tx: &mut Tx) -> bool {
        match ev.key {
            Key::Command('f') => tx.focus("snapshot-search", false),
            Key::Escape if self.pending_delete.is_some() => self.pending_delete = None,
            Key::Escape if !self.query.text.is_empty() => {
                self.query = TextEdit::new("");
                self.query.focused = true;
                self.filter(tx);
            }
            Key::Command('r') => self.resume(),
            Key::Down | Key::Up => {
                let entries: Vec<_> = self.filtered.iter().map(|&i| &self.entries[i]).collect();
                if entries.is_empty() {
                    return true;
                }
                let index = entries.iter().position(|s| Some(&s.id) == self.selected.as_ref());
                let next = match (ev.key == Key::Down, index) {
                    (true, Some(i)) => (i + 1).min(entries.len() - 1),
                    (false, Some(i)) => i.saturating_sub(1),
                    _ => 0,
                };
                self.selected = Some(entries[next].id.clone());
                self.pending_delete = None;
                tx.scroll_into_view(format!("snapshot-{}", entries[next].id));
            }
            _ => return false,
        }
        true
    }

    /// Lists `entries` as if the writer had just read them from storage.
    #[cfg(test)]
    pub(crate) fn seed(&mut self, entries: Vec<Snapshot>, tx: &mut Tx) {
        self.loaded = true;
        self.search = entries.iter().map(Snapshot::search_text).collect();
        self.entries = entries;
        self.filter(tx);
    }

    /// How many snapshots are saved, of how many storage keeps, for the page's title band.
    pub fn count(&self, k: &Kit) -> Option<El> {
        self.loaded.then(|| {
            text(format!(
                "{} of {} saved",
                self.entries.len(),
                dupin_core::snapshots::MAX_ENTRIES
            ))
            .key("snapshot-count")
            .font(k.ty.small)
            .color(k.c.text_muted)
        })
    }
}

/// What a snapshot holds, as the page names it.
fn kind(saved: &Snapshot) -> &'static str {
    if saved.mode == crate::features::calculator::CALCULATOR
        || matches!(saved.views.first(), Some(dupin_core::View::Pair { .. }))
    {
        "Calculation"
    } else if saved.mode.starts_with("extension.") || saved.mode.starts_with("extension-search.") {
        "Extension view"
    } else {
        "Search"
    }
}

/// Settings' Snapshots page: a search over the saved snapshots and their list on the left, the
/// selected one's saved content and actions on the right.
pub fn page(s: &Snapshots, k: &Kit, now: u64) -> El {
    let c = &k.c;
    let mut list = col()
        .key("snapshot-list")
        .role("list")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(2.0)
        .scroll(ScrollSpec { stick_bottom: false });
    for saved in s.filtered.iter().map(|&i| &s.entries[i]) {
        let id = saved.id.clone();
        let selected = s.selected.as_ref() == Some(&id);
        list = list.child(
            col()
                .key(format!("snapshot-{id}"))
                .role("listitem")
                .label(&saved.title)
                .focusable(true)
                .focus_selected(selected)
                .focus_ring(2.0, 0.0, c.border_focus)
                .w_full()
                .px(10.0)
                .py(8.0)
                .gap(2.0)
                .radius(k.radius.sm)
                .hover(c.hover)
                .press(c.pressed)
                .fill(if selected { c.layer_02 } else { c.bg_base })
                .on_press(move |p: &mut Prefs| {
                    p.snapshots.open(&id);
                })
                .child(
                    text(&saved.title)
                        .font(k.ty.ui_medium)
                        .color(c.text_base)
                        .truncate(),
                )
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .child(text(kind(saved)).font(k.ty.small).color(c.text_muted).truncate())
                        .child(el().grow(1.0))
                        .child(
                            text(saved_ago(now, saved.saved_at))
                                .font(k.ty.small)
                                .color(c.text_muted),
                        ),
                ),
        );
    }
    if s.filtered.is_empty() && !s.query.text.is_empty() {
        list = list.child(
            text("No matches for this search.")
                .px(10.0)
                .py(8.0)
                .font(k.ty.small)
                .color(c.text_muted)
                .wrap(),
        );
    }
    let left = col()
        .key("snapshot-sidebar")
        .w(LIST_WIDTH)
        .h_full()
        .gap(8.0)
        .child(
            text_input(&s.query, k.ty.ui)
                .key("snapshot-search")
                .label("Search saved snapshots")
                .single_line()
                .placeholder("Search snapshots", c.text_muted)
                .color(c.text_base)
                .px(8.0)
                .py(6.0)
                .radius(k.radius.sm)
                .fill(c.layer_02)
                .on_edit(|p: &mut Prefs| &mut p.snapshots.query)
                .on_input_change(|p: &mut Prefs, tx: &mut Tx| p.snapshots.filter(tx))
                .on_input_key(|p: &mut Prefs, ev: &KeyEvent, tx: &mut Tx| p.snapshots.key(ev, tx)),
        )
        .child(list);
    row()
        .key("snapshots")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(20.0)
        .child(left)
        .child(detail(s, k))
}

fn detail(s: &Snapshots, k: &Kit) -> El {
    let c = &k.c;
    let mut detail = col()
        .key("snapshot-detail")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .gap(12.0)
        .scroll(ScrollSpec { stick_bottom: false });
    let problem = s.archive_problem.as_deref().or(s.problem.as_deref());
    detail = detail.children(problem.map(|p| text(p).key("snapshot-problem").color(c.text_base).wrap()));
    let Some(saved) = s
        .selected
        .as_deref()
        .and_then(|id| s.entries.iter().find(|saved| saved.id == id))
    else {
        let (title, hint) = if s.archive_problem.is_some() {
            (
                "Snapshots are unavailable",
                "Saved snapshots stay unreadable until their encrypted storage can be opened.",
            )
        } else if s.entries.is_empty() {
            (
                "No snapshots yet",
                "Save a settled calculation, search or extension view with ⇧⌘S in the launcher.",
            )
        } else {
            ("", "Select a snapshot.")
        };
        return detail.child(
            col()
                .key("snapshot-empty")
                .w_full()
                .grow(1.0)
                .center()
                .gap(6.0)
                .children((!title.is_empty()).then(|| text(title).font(k.ty.body_medium).color(c.text_base)))
                .child(text(hint).font(k.ty.small).color(c.text_muted).wrap()),
        );
    };
    let id = saved.id.clone();
    let available = s.available(&saved.mode);
    let pending_delete = s.pending_delete.as_deref() == Some(id.as_str());
    let delete_id = id.clone();
    detail
        .child(
            text("Historical snapshot · saved content")
                .font(k.ty.small)
                .color(c.text_muted),
        )
        .child(
            text(&saved.title)
                .font(rune::TextStyle {
                    size: 20.0,
                    ..k.ty.ui_medium
                })
                .color(c.text_base)
                .wrap()
                .selectable(true),
        )
        .child(
            text(format!(
                "{} · {} · {} · Saved {}",
                kind(saved),
                source_name(&saved.mode),
                saved.query,
                saved_date(saved.saved_at)
            ))
            .font(k.ty.small)
            .color(c.text_muted)
            .wrap(),
        )
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(
                    button("Open in Panel", rune_kit::ButtonVariant::Submit, !available, k)
                        .on_press(|p: &mut Prefs| p.snapshots.resume()),
                )
                .child(
                    button("Copy", rune_kit::ButtonVariant::Neutral, false, k)
                        .on_press_tx(|p: &mut Prefs, tx: &mut Tx| p.snapshots.copy(tx)),
                )
                .child(
                    button("Delete…", rune_kit::ButtonVariant::Neutral, false, k)
                        .on_press(move |p: &mut Prefs| p.snapshots.pending_delete = Some(delete_id.clone())),
                )
                .children((s.copied.as_deref() == Some(id.as_str())).then(|| {
                    text("Copied")
                        .key("snapshot-copied")
                        .font(k.ty.small)
                        .color(c.text_muted)
                })),
        )
        .children((!available).then(|| {
            text("Source unavailable. Saved content remains readable.")
                .font(k.ty.small)
                .color(c.text_muted)
        }))
        .children(pending_delete.then(|| {
            col()
                .gap(8.0)
                .child(
                    text("Delete this saved snapshot? This cannot be undone.")
                        .wrap()
                        .color(c.text_base),
                )
                .child(
                    row()
                        .gap(8.0)
                        .child(
                            button("Cancel", rune_kit::ButtonVariant::Neutral, false, k)
                                .on_press(|p: &mut Prefs| p.snapshots.pending_delete = None),
                        )
                        .child(
                            button("Delete snapshot", rune_kit::ButtonVariant::Neutral, false, k)
                                .on_press(move |p: &mut Prefs| p.snapshots.delete(&id)),
                        ),
                )
        }))
        .children(saved_views(&saved.views, k))
}

/// A snapshot's saved views as cards, as they were shown when it was saved.
fn saved_views<'a>(views: &'a [dupin_core::View], k: &'a Kit) -> impl Iterator<Item = El> + 'a {
    views.iter().map(|view| {
        let card = col()
            .w_full()
            .gap(6.0)
            .padding(12.0)
            .radius(k.radius.sm)
            .fill(k.c.layer_02);
        match view {
            dupin_core::View::Text { title, subtitle } => card
                .child(
                    text(title)
                        .font(k.ty.ui_medium)
                        .color(k.c.text_base)
                        .wrap()
                        .selectable(true),
                )
                .child(
                    text(subtitle)
                        .font(k.ty.small)
                        .color(k.c.text_muted)
                        .wrap()
                        .selectable(true),
                ),
            dupin_core::View::Detail { title, body } => card
                .child(
                    text(title)
                        .font(k.ty.body_medium)
                        .color(k.c.text_base)
                        .wrap()
                        .selectable(true),
                )
                .child(
                    text(body)
                        .font(k.ty.body)
                        .color(k.c.text_base)
                        .wrap()
                        .selectable(true),
                ),
            dupin_core::View::Pair { source, result } => card
                .child(
                    text(format!("{source} →"))
                        .font(k.ty.small)
                        .color(k.c.text_muted)
                        .wrap()
                        .selectable(true),
                )
                .child(
                    text(result)
                        .font(k.ty.body_medium)
                        .color(k.c.text_base)
                        .wrap()
                        .selectable(true),
                ),
        }
    })
}

/// A snapshot's saved views as plain text, one paragraph each, for the clipboard.
fn plain_text(views: &[dupin_core::View]) -> String {
    views
        .iter()
        .map(|view| match view {
            dupin_core::View::Text { title, subtitle } if subtitle.is_empty() => title.clone(),
            dupin_core::View::Text { title, subtitle } => format!("{title}\n{subtitle}"),
            dupin_core::View::Detail { title, body } => format!("{title}\n{body}"),
            dupin_core::View::Pair { source, result } => format!("{source} → {result}"),
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

enum Request {
    Save(Snapshot, crate::ui::Busy),
    Delete(String, crate::ui::Busy),
}

fn publish(
    store: &Store,
    prefs: &AppHandle<Prefs>,
    result: Option<Result<Option<String>, String>>,
    cache: &mut std::collections::HashMap<String, String>,
) {
    match &result {
        Some(Ok(Some(_))) => crate::trace::snapshot("saved"),
        Some(Ok(None)) => crate::trace::snapshot("deleted"),
        Some(Err(_)) => crate::trace::snapshot("write failed"),
        None => {}
    }
    let entries = store.list().to_vec();
    cache.retain(|id, _| entries.iter().any(|entry| &entry.id == id));
    let search = entries
        .iter()
        .map(|entry| {
            cache
                .entry(entry.id.clone())
                .or_insert_with(|| entry.search_text())
                .clone()
        })
        .collect();
    let problem = store.problem().map(str::to_owned);
    listing().list(&entries);
    prefs.update(move |prefs, tx| {
        let s = &mut prefs.snapshots;
        s.loaded = true;
        s.entries = entries;
        s.search = search;
        s.archive_problem = problem;
        if let Some(result) = result {
            match result {
                Ok(Some(id)) => {
                    s.selected = Some(id);
                    s.problem = None;
                    s.panel.update(|ui, tx| {
                        ui.snapshot_notice("Snapshot saved. Open Snapshots with ⌘⇧O.".into(), tx)
                    });
                }
                Ok(None) => {
                    s.problem = None;
                }
                Err(problem) => {
                    s.problem = Some(problem.clone());
                    s.panel.update(move |ui, tx| ui.snapshot_notice(problem, tx));
                }
            }
        }
        s.filter(tx);
        s.panel.update(crate::ui::Ui::refresh_snapshots);
    });
}

fn summaries(entries: &[Snapshot], problem: Option<String>, budget: usize) -> crate::control::SnapshotList {
    let mut list = crate::control::SnapshotList {
        entries: Vec::new(),
        truncated: false,
        problem,
    };
    let mut ordered: Vec<_> = entries.iter().collect();
    ordered.sort_by_key(|s| std::cmp::Reverse(s.saved_at));
    for entry in ordered {
        list.entries.push(crate::control::SnapshotSummary {
            id: entry.id.clone(),
            saved_at: entry.saved_at,
            mode: entry.mode.clone(),
            title: entry.title.clone(),
            query: entry.query.clone(),
        });
        if serde_json::to_vec(&list)
            .expect("snapshot metadata encodes")
            .len()
            > budget
        {
            list.entries.pop();
            list.truncated = true;
            break;
        }
    }
    list
}

/// What the Snapshots page shows of one snapshot: its metadata and visible views, never its
/// resume state. Views that would not fit in `budget` bytes are left out and counted.
fn content(entry: &Snapshot, budget: usize) -> serde_json::Value {
    let mut value = serde_json::json!({
        "id": entry.id,
        "saved_at": entry.saved_at,
        "mode": entry.mode,
        "title": entry.title,
        "query": entry.query,
        "views": [],
        "truncated": false,
        "omitted_views": 0,
    });
    let mut size = serde_json::to_vec(&value)
        .expect("snapshot content encodes")
        .len();
    let mut views = Vec::new();
    for view in &entry.views {
        let view = serde_json::to_value(view).expect("snapshot view encodes");
        size += view.to_string().len() + 1;
        if size > budget {
            break;
        }
        views.push(view);
    }
    value["omitted_views"] = (entry.views.len() - views.len()).into();
    value["truncated"] = (views.len() < entry.views.len()).into();
    value["views"] = views.into();
    value
}

/// What the `snap` keyword lists: each saved snapshot's metadata, newest first, as the
/// Snapshots page last loaded it, and the commands a snapshot may resume. Only the metadata the
/// page's own list shows is kept here, never saved content.
pub struct Listing {
    entries: Option<Vec<crate::control::SnapshotSummary>>,
    pub sources: Vec<String>,
}

impl Listing {
    fn list(&mut self, saved: &[Snapshot]) {
        self.entries = Some(summaries(saved, None, usize::MAX).entries);
    }

    /// The saved snapshots, as [`read_listing`] or the Snapshots page last left them.
    pub fn entries(&self) -> &[crate::control::SnapshotSummary] {
        self.entries.as_deref().unwrap_or_default()
    }
}

/// Reads the saved snapshots' metadata from the store if the Snapshots page hasn't yet. It
/// takes the store's lock and the listing's one after the other, never together, since the
/// writer takes them in the other order.
pub fn read_listing() {
    if listing().entries.is_some() {
        return;
    }
    let saved = summaries(store::lock(store::shared()).list(), None, usize::MAX).entries;
    listing().entries.get_or_insert(saved);
}

pub fn listing() -> std::sync::MutexGuard<'static, Listing> {
    static LISTING: std::sync::Mutex<Listing> = std::sync::Mutex::new(Listing {
        entries: None,
        sources: Vec::new(),
    });
    LISTING.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub fn source_name(mode: &str) -> &str {
    match mode {
        "apps.search" => "Applications",
        "calculator.evaluate" => "Calculator",
        "files.names" => "Files",
        "files.contents" => "File contents",
        "dictionary.lookup" => "Dictionary",
        "settings.search" => "Settings",
        "web.keywords" | "web.fallbacks" => "Web search",
        _ => mode,
    }
}

fn button(label: &str, variant: rune_kit::ButtonVariant, disabled: bool, k: &Kit) -> El {
    rune_kit::Button::new(label)
        .variant(variant)
        .disabled(disabled)
        .build(k)
        .key(label)
}

/// How long before `now` the Unix second `then` was, as the page's list says it: relative within
/// a week, else the day.
pub fn saved_ago(now: u64, then: u64) -> String {
    match now.saturating_sub(then) {
        0..60 => "Just now".into(),
        elapsed @ 60..604_800 => dupin_core::ago_ms(elapsed * 1000, 0),
        _ => saved_date(then).split(' ').next().unwrap_or_default().to_owned(),
    }
}

pub fn saved_date(seconds: u64) -> String {
    let Ok(seconds) = libc::time_t::try_from(seconds) else {
        return "unknown date".into();
    };
    let mut date = std::mem::MaybeUninit::<libc::tm>::uninit();
    // gmtime_r writes only to the supplied storage and has no shared calendar state.
    if unsafe { libc::gmtime_r(&seconds, date.as_mut_ptr()) }.is_null() {
        return "unknown date".into();
    }
    let date = unsafe { date.assume_init() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC",
        date.tm_year + 1900,
        date.tm_mon + 1,
        date.tm_mday,
        date.tm_hour,
        date.tm_min
    )
}

fn write_requests(
    store: &std::sync::Mutex<Store>,
    rx: std::sync::mpsc::Receiver<Request>,
    mut emit: impl FnMut(&Store, Option<Result<Option<String>, String>>),
) {
    emit(&store::lock(store), None);
    while let Ok(request) = rx.recv() {
        let mut store = store::lock(store);
        let (result, busy) = match request {
            Request::Save(saved, busy) => (store.save(saved).map(Some), busy),
            Request::Delete(id, busy) => (store.delete(&id).map(|_| None), busy),
        };
        drop(busy);
        emit(&store, Some(result));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(id: &str, saved_at: u64) -> Snapshot {
        Snapshot {
            version: 1,
            id: id.into(),
            saved_at,
            mode: "apps.search".into(),
            explicit: false,
            query: "query".into(),
            title: "Saved".into(),
            views: vec![dupin_core::View::Text {
                title: "Private content".into(),
                subtitle: String::new(),
            }],
            state: None,
        }
    }

    #[test]
    fn summary_metadata_is_ordered_private_and_frame_bounded() {
        let entries = vec![saved("older", 1), saved("newer", 2)];
        let list = summaries(
            &entries,
            Some("Archive problem".into()),
            crate::control::MAX_FRAME / 2,
        );
        assert_eq!(list.entries[0].id, "newer");
        assert_eq!(list.problem.as_deref(), Some("Archive problem"));
        let json = serde_json::to_value(&list).unwrap();
        for entry in json["entries"].as_array().unwrap() {
            assert_eq!(entry.as_object().unwrap().len(), 5);
            for key in ["views", "state", "explicit"] {
                assert!(entry.get(key).is_none());
            }
        }
        let entries: Vec<_> = (0..100)
            .map(|i| {
                let mut entry = saved(&i.to_string(), i);
                entry.title = "\"".repeat(4096);
                entry.query = "\\".repeat(4096);
                entry
            })
            .collect();
        let list = summaries(&entries, None, crate::control::MAX_FRAME / 2);
        assert!(list.truncated);
        crate::control::write_frame(
            &mut Vec::new(),
            &crate::control::Reply {
                id: 1,
                result: Ok(serde_json::to_value(list).unwrap()),
            },
        )
        .unwrap();
    }

    #[test]
    fn read_content_has_the_visible_views_and_no_state_and_fits_a_frame() {
        let mut entry = saved("a", 5);
        entry.state = Some(dupin_plugin_protocol::State {
            version: 1,
            data: serde_json::json!({"secret": 1}),
        });
        entry.views.push(dupin_core::View::Pair {
            source: "2 + 2".into(),
            result: "4".into(),
        });
        let value = content(&entry, crate::control::MAX_FRAME - 1024);
        assert_eq!(value["views"][0]["kind"], "text");
        assert_eq!(value["views"][0]["title"], "Private content");
        assert_eq!(value["views"][1]["result"], "4");
        assert_eq!(value["truncated"], false);
        assert_eq!(value["omitted_views"], 0);
        for key in ["state", "explicit", "version"] {
            assert!(value.get(key).is_none());
        }

        entry.views = (0..3)
            .map(|_| dupin_core::View::Detail {
                title: "t".into(),
                body: "\"".repeat(300 * 1024),
            })
            .collect();
        let value = content(&entry, crate::control::MAX_FRAME - 1024);
        assert_eq!(value["truncated"], true);
        assert_eq!(value["views"].as_array().unwrap().len(), 0);
        assert_eq!(value["omitted_views"], 3);
        entry.views[0] = dupin_core::View::Detail {
            title: "t".into(),
            body: "x".repeat(100 * 1024),
        };
        let value = content(&entry, crate::control::MAX_FRAME - 1024);
        assert_eq!(value["views"].as_array().unwrap().len(), 1);
        assert_eq!(value["omitted_views"], 2);
        crate::control::write_frame(
            &mut Vec::new(),
            &crate::control::Reply {
                id: 1,
                result: Ok(value),
            },
        )
        .unwrap();
    }

    #[test]
    fn snapshot_control_never_writes_or_resumes() {
        use crate::control::{Call, Command, Request as ControlRequest};
        let panel = rune::App::new(rune::AppSpec::new(
            "panel",
            (1.0, 1.0),
            || unreachable!(),
            crate::view::view,
        ))
        .handle();
        let (writer, received) = std::sync::mpsc::channel();
        let mut s = Snapshots {
            loaded: false,
            sources: None,
            entries: vec![saved("other", 2), saved("saved", 1)],
            filtered: vec![0, 1],
            search: vec!["query".into(), "query".into()],
            writer,
            archive_problem: None,
            query: TextEdit::new("query"),
            selected: Some("other".into()),
            pending_delete: Some("other".into()),
            copied: None,
            problem: None,
            panel,
            running: Default::default(),
        };
        let request = |command| {
            let (reply, receiver) = std::sync::mpsc::sync_channel(1);
            let call = Call {
                request: ControlRequest { id: 1, command },
                cancelled: Default::default(),
                reply,
            };
            (call, receiver)
        };
        for command in [
            Command::SnapshotsList {},
            Command::SnapshotsRead { id: "saved".into() },
        ] {
            let (call, reply) = request(command);
            s.control(call);
            assert_eq!(
                reply.recv().unwrap().result.unwrap_err(),
                "Busy: snapshots loading"
            );
        }
        s.loaded = true;
        for (command, expected) in [
            (
                Command::SnapshotsOpen {
                    id: Some("missing".into()),
                },
                Some("NotFound: snapshot"),
            ),
            (
                Command::SnapshotsRead { id: "missing".into() },
                Some("NotFound: snapshot"),
            ),
            (Command::SnapshotsList {}, None),
            (Command::SnapshotsRead { id: "saved".into() }, None),
        ] {
            let (call, reply) = request(command);
            s.control(call);
            assert_eq!(reply.recv().unwrap().result.err().as_deref(), expected);
            assert_eq!(s.selected.as_deref(), Some("other"));
        }
        let open = || {
            request(Command::SnapshotsOpen {
                id: Some("saved".into()),
            })
        };
        let (call, reply) = open();
        s.control(call);
        assert!(
            matches!(reply.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)),
            "opening replies only after the panel's final Busy check"
        );
        assert_eq!(s.selected.as_deref(), Some("other"));
        assert_eq!(s.query.text, "query");
        assert_eq!(s.pending_delete.as_deref(), Some("other"));

        let removed = s.entries.pop().unwrap();
        let search = s.search.pop().unwrap();
        let mut tx = Tx::default();
        s.filter(&mut tx);
        let (call, reply) = open();
        s.accept_open(call, &mut tx);
        assert_eq!(reply.recv().unwrap().result.unwrap_err(), "NotFound: snapshot");
        assert_eq!(s.selected.as_deref(), Some("other"));
        assert_eq!(s.query.text, "query");
        assert_eq!(s.pending_delete.as_deref(), Some("other"));
        s.entries.push(removed);
        s.search.push(search);

        let (call, reply) = open();
        s.accept_open(call, &mut tx);
        assert_eq!(reply.recv().unwrap().result.unwrap(), serde_json::Value::Null);
        assert_eq!(s.selected.as_deref(), Some("saved"));
        assert!(s.query.text.is_empty());
        assert!(s.pending_delete.is_none());

        assert!(received.try_recv().is_err());
    }

    #[test]
    fn writer_reports_success_failure_and_preserves_saved_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let key = dupin_storage::DataKey::generate().unwrap();
        let storage = |key: Box<dyn dupin_storage::KeyProvider>| {
            let storage = dupin_storage::Storage::new(dir.path(), BoxedKey(key));
            Some(std::sync::Arc::new(storage))
        };
        let (send, rx) = std::sync::mpsc::channel();
        let (reply, results) = std::sync::mpsc::channel();
        let running = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let ready = std::sync::Arc::new(std::sync::Barrier::new(2));
        let worker_ready = ready.clone();
        let store = std::sync::Mutex::new(Store::open(storage(Box::new(dupin_storage::MemoryKey::new(
            key.clone(),
        )))));
        let thread = std::thread::spawn(move || {
            write_requests(&store, rx, |store, result| {
                let loading = result.is_none();
                reply.send((store.list().to_vec(), result)).unwrap();
                if loading {
                    worker_ready.wait();
                }
            });
        });
        assert!(results.recv().unwrap().0.is_empty());
        let saved = Snapshot {
            version: 1,
            id: String::new(),
            saved_at: 0,
            mode: "apps.search".into(),
            explicit: false,
            query: "2+2".into(),
            title: "Calculation".into(),
            views: vec![dupin_core::View::Pair {
                source: "2+2".into(),
                result: "4".into(),
            }],
            state: None,
        };
        send.send(Request::Save(
            saved.clone(),
            crate::ui::Busy::new(running.clone()),
        ))
        .ok()
        .unwrap();
        assert_eq!(running.load(std::sync::atomic::Ordering::Acquire), 1);
        ready.wait();
        let (entries, result) = results.recv().unwrap();
        assert_eq!(running.load(std::sync::atomic::Ordering::Acquire), 0);
        let id = result.unwrap().unwrap().unwrap();
        assert_eq!(entries[0].id, id);
        assert!(entries[0].saved_at > 0);
        send.send(Request::Delete(id, crate::ui::Busy::new(running.clone())))
            .ok()
            .unwrap();
        let (entries, result) = results.recv().unwrap();
        assert!(entries.is_empty());
        assert_eq!(result.unwrap().unwrap(), None);
        assert_eq!(running.load(std::sync::atomic::Ordering::Acquire), 0);
        send.send(Request::Save(
            saved.clone(),
            crate::ui::Busy::new(running.clone()),
        ))
        .ok()
        .unwrap();
        let (kept, _) = results.recv().unwrap();
        drop(send);
        thread.join().unwrap();

        // The Keychain refuses the key: the save fails, its work is released and what was saved
        // stays sealed as it was.
        let (send, rx) = std::sync::mpsc::channel();
        send.send(Request::Save(saved, crate::ui::Busy::new(running.clone())))
            .ok()
            .unwrap();
        drop(send);
        let failing = std::sync::Mutex::new(Store::open(storage(Box::new(NoKey))));
        let mut outcomes = Vec::new();
        write_requests(&failing, rx, |store, result| {
            outcomes.push((store.list().len(), store.problem().is_some(), result));
        });
        assert!(matches!(
            outcomes.as_slice(),
            [(0, true, None), (0, true, Some(Err(_)))]
        ));
        assert_eq!(running.load(std::sync::atomic::Ordering::Acquire), 0);
        let reopened = Store::open(storage(Box::new(dupin_storage::MemoryKey::new(key))));
        assert_eq!(reopened.list(), kept.as_slice());
    }

    struct BoxedKey(Box<dyn dupin_storage::KeyProvider>);

    impl dupin_storage::KeyProvider for BoxedKey {
        fn data_key(&self) -> Result<dupin_storage::DataKey, dupin_storage::Error> {
            self.0.data_key()
        }
    }

    struct NoKey;

    impl dupin_storage::KeyProvider for NoKey {
        fn data_key(&self) -> Result<dupin_storage::DataKey, dupin_storage::Error> {
            Err(dupin_storage::Error::Key("the Keychain is locked".into()))
        }
    }

    #[test]
    fn the_page_names_each_kind_says_when_and_copies_plain_text() {
        let mut calculation = saved("sum", 1);
        calculation.views = vec![dupin_core::View::Pair {
            source: "2 + 2".into(),
            result: "4".into(),
        }];
        let mut extension = saved("words", 1);
        extension.mode = "extension.word-count".into();
        assert_eq!(kind(&calculation), "Calculation");
        assert_eq!(kind(&extension), "Extension view");
        assert_eq!(kind(&saved("apps", 1)), "Search");

        let now = 1_795_746_000;
        assert_eq!(saved_ago(now, now - 5), "Just now");
        assert_eq!(saved_ago(now, now - 3 * 60), "3 min ago");
        assert_eq!(saved_ago(now, now - 2 * 86_400), "2 d ago");
        assert_eq!(saved_ago(now, 1_795_082_400), "2026-11-19");

        assert_eq!(
            plain_text(&[
                dupin_core::View::Pair {
                    source: "2 + 2".into(),
                    result: "4".into(),
                },
                dupin_core::View::Text {
                    title: "Title".into(),
                    subtitle: String::new(),
                },
                dupin_core::View::Detail {
                    title: "Word".into(),
                    body: "Meaning".into(),
                },
            ]),
            "2 + 2 → 4\n\nTitle\n\nWord\nMeaning"
        );
    }

    #[test]
    fn unavailable_writer_releases_pending_work() {
        let running = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (send, receive) = std::sync::mpsc::channel();
        drop(receive);
        let request = Request::Delete("fixture".into(), crate::ui::Busy::new(running.clone()));
        assert_eq!(running.load(std::sync::atomic::Ordering::Acquire), 1);
        drop(send.send(request));
        assert_eq!(running.load(std::sync::atomic::Ordering::Acquire), 0);
    }
    #[test]
    fn focused_search_command_backspace_and_confirmation_cancel_never_delete() {
        let panel = rune::App::new(rune::AppSpec::new(
            "panel",
            (1.0, 1.0),
            || unreachable!(),
            crate::view::view,
        ))
        .handle();
        let (writer, received) = std::sync::mpsc::channel();
        let mut s = Snapshots {
            loaded: true,
            sources: None,
            entries: vec![],
            filtered: vec![],
            search: vec![],
            writer,
            archive_problem: None,
            query: TextEdit::new("query"),
            selected: Some("saved".into()),
            pending_delete: Some("saved".into()),
            copied: None,
            problem: None,
            panel,
            running: Default::default(),
        };
        s.query.focused = true;
        let mut tx = Tx::default();
        assert!(!s.key(
            &KeyEvent {
                key: Key::Backspace,
                mods: rune::Mods {
                    cmd: true,
                    ..Default::default()
                }
            },
            &mut tx
        ));
        assert_eq!(s.selected.as_deref(), Some("saved"));
        assert_eq!(s.query.text, "query");
        s.key(
            &KeyEvent {
                key: Key::Escape,
                mods: Default::default(),
            },
            &mut tx,
        );
        assert!(s.pending_delete.is_none());
        assert_eq!(s.selected.as_deref(), Some("saved"));
        assert!(received.try_recv().is_err());
    }
}
