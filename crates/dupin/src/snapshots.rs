use dupin_core::snapshots::{Snapshot, Store};
use rune::{AppHandle, Cx, El, Key, KeyEvent, ScrollSpec, TextEdit, Tx, col, row, text, text_input};

pub struct Snapshots {
    me: AppHandle<Self>,
    loaded: bool,
    sources: Vec<String>,
    entries: Vec<Snapshot>,
    filtered: Vec<usize>,
    search: Vec<String>,
    writer: std::sync::mpsc::Sender<Request>,
    archive_problem: Option<String>,
    query: TextEdit,
    selected: Option<String>,
    pending_delete: Option<String>,
    problem: Option<String>,
    panel: AppHandle<crate::ui::Ui>,
    running: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Snapshots {
    pub fn new(
        panel: AppHandle<crate::ui::Ui>,
        me: AppHandle<Self>,
        running: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) -> Self {
        let (writer, rx) = std::sync::mpsc::channel();
        let handle = me.clone();
        std::thread::Builder::new()
            .name("dupin-snapshots".into())
            .spawn(move || {
                let mut store = match std::env::var_os("XDG_DATA_HOME").or_else(|| std::env::var_os("HOME")) {
                    Some(base) => Store::from_base(
                        base.into(),
                        std::path::Path::new(if std::env::var_os("XDG_DATA_HOME").is_some() {
                            "dupin/snapshots"
                        } else {
                            "Library/Application Support/Dupin/snapshots"
                        }),
                    ),
                    None => Store::from_base(std::path::PathBuf::new(), std::path::Path::new("")),
                };
                let mut cache = std::collections::HashMap::new();
                write_requests(&mut store, rx, |store, result| {
                    publish(store, &me, result, &mut cache)
                });
            })
            .expect("spawn snapshot writer");
        let mut query = TextEdit::new("");
        query.focused = true;
        Self {
            me: handle,
            loaded: false,
            sources: Vec::new(),
            entries: Vec::new(),
            filtered: Vec::new(),
            search: Vec::new(),
            writer,
            archive_problem: None,
            query,
            selected: None,
            pending_delete: None,
            problem: None,
            panel,
            running,
        }
    }

    pub fn set_sources(&mut self, sources: Vec<String>) {
        self.sources = sources;
    }

    pub fn control(&mut self, call: crate::control::Call, tx: &mut Tx) {
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
            Command::SnapshotsOpen { id } => {
                if id.is_some() && !self.loaded {
                    Err("Busy: snapshots loading".into())
                } else if id
                    .as_ref()
                    .is_some_and(|id| !self.entries.iter().any(|s| &s.id == id))
                {
                    Err("NotFound: snapshot".into())
                } else {
                    self.query = TextEdit::new("");
                    self.query.focused = true;
                    self.pending_delete = None;
                    self.filter();
                    if let Some(id) = id {
                        self.open(id);
                    }
                    self.panel
                        .update(move |ui, tx| ui.control_snapshots_open(call, tx));
                    return;
                }
            }
            Command::SnapshotsClose {} => {
                self.pending_delete = None;
                if self.me.is_visible() {
                    tx.close_window();
                }
                Ok(serde_json::Value::Null)
            }
            _ => Err("invalid snapshots command".into()),
        };
        call.finish(result);
    }

    fn filter(&mut self) {
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
        if self.entries.iter().find(|s| s.id == id).is_none() {
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
        if let Some(saved) = self
            .selected
            .as_deref()
            .and_then(|id| self.entries.iter().find(|s| s.id == id))
            .cloned()
        {
            if !self.sources.contains(&saved.mode) {
                self.report("The source command is unavailable. Saved content remains readable.".into());
                return;
            }
            self.panel.update(move |ui, tx| ui.resume_snapshot(saved, tx));
        }
    }

    pub fn closed(&mut self, _: &mut Tx) {
        self.panel.update(crate::ui::Ui::resume_restart);
    }

    pub fn key(&mut self, ev: &KeyEvent, tx: &mut Tx) -> bool {
        match ev.key {
            Key::Command('f') => tx.focus("snapshot-search", false),
            Key::Escape if self.pending_delete.is_some() => self.pending_delete = None,
            Key::Escape if !self.query.text.is_empty() => {
                self.query = TextEdit::new("");
                self.query.focused = true;
                self.filter();
            }
            Key::Escape => tx.close_window(),
            Key::Command('r') => self.resume(),
            Key::Command('w') => tx.close_window(),
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
                tx.scroll_into_view(format!("snapshot-{}", entries[next].id));
            }
            _ => return false,
        }
        true
    }
}

pub fn view(s: &Snapshots, cx: &mut Cx<Snapshots>) -> El {
    let k = rune_theme_opencode::kit(cx.system_dark());
    let mut list = col()
        .w(240.0)
        .h_full()
        .gap(4.0)
        .scroll(ScrollSpec { stick_bottom: false });
    let mut count = 0;
    for saved in s.filtered.iter().map(|&i| &s.entries[i]) {
        count += 1;
        let id = saved.id.clone();
        let selected = s.selected.as_ref() == Some(&id);
        list = list.child(
            col()
                .gap(4.0)
                .child(
                    text(&saved.title)
                        .font(k.ty.ui_medium)
                        .color(k.c.text_base)
                        .wrap(),
                )
                .child(
                    text(format!(
                        "{} · {} · {}",
                        source_name(&saved.mode),
                        saved.query,
                        saved_date(saved.saved_at)
                    ))
                    .font(k.ty.small)
                    .color(k.c.text_muted)
                    .wrap(),
                )
                .key(format!("snapshot-{id}"))
                .label(&saved.title)
                .padding(10.0)
                .focus_selected(selected)
                .hover(k.c.hover)
                .fill(if selected { k.c.layer_02 } else { k.c.bg_base })
                .on_press(move |s: &mut Snapshots| {
                    s.open(&id);
                }),
        );
    }
    if count == 0 {
        list = list.child(
            text(if s.archive_problem.as_deref().is_some() {
                "Saved snapshots are unavailable until the archive problem is resolved."
            } else {
                if s.query.text.is_empty() {
                    "No snapshots yet. Save a settled search with ⌘⇧S."
                } else {
                    "No matches for this search."
                }
            })
            .wrap()
            .color(k.c.text_muted),
        );
    }
    let mut detail = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(12.0)
        .scroll(ScrollSpec { stick_bottom: false });
    if let Some(saved) = s
        .selected
        .as_deref()
        .and_then(|id| s.entries.iter().find(|saved| saved.id == id))
    {
        let id = saved.id.clone();
        let pending_delete = s.pending_delete.as_deref() == Some(id.as_str());
        let delete_id = id.clone();
        detail = detail
            .child(
                text("Historical snapshot · saved content")
                    .font(k.ty.small)
                    .color(k.c.text_muted),
            )
            .child(
                text(&saved.title)
                    .font(rune::TextStyle {
                        size: 22.0,
                        ..k.ty.ui_medium
                    })
                    .color(k.c.text_base)
                    .wrap(),
            )
            .child(
                text(format!(
                    "{} · {} · Saved {}",
                    source_name(&saved.mode),
                    saved.query,
                    saved_date(saved.saved_at)
                ))
                .font(k.ty.small)
                .color(k.c.text_muted)
                .wrap(),
            )
            .child(
                row()
                    .gap(20.0)
                    .child(
                        button("Run again (⌘R)", k)
                            .disabled(!s.sources.contains(&saved.mode))
                            .color(k.c.text_base)
                            .focusable(true)
                            .on_press(|s: &mut Snapshots| s.resume()),
                    )
                    .child(
                        button("Delete…", k)
                            .color(k.c.text_base)
                            .focusable(true)
                            .on_press(move |s: &mut Snapshots| s.pending_delete = Some(delete_id.clone())),
                    ),
            )
            .children((!s.sources.contains(&saved.mode)).then(|| {
                text("Source unavailable. Saved content remains readable.")
                    .font(k.ty.small)
                    .color(k.c.text_muted)
            }))
            .children(pending_delete.then(|| {
                col()
                    .gap(8.0)
                    .child(
                        text("Delete this saved snapshot? This cannot be undone.")
                            .wrap()
                            .color(k.c.text_base),
                    )
                    .child(
                        row()
                            .gap(20.0)
                            .child(
                                button("Cancel", k)
                                    .focusable(true)
                                    .color(k.c.text_base)
                                    .on_press(|s: &mut Snapshots| s.pending_delete = None),
                            )
                            .child(
                                button("Delete snapshot", k)
                                    .focusable(true)
                                    .color(k.c.text_base)
                                    .on_press(move |s: &mut Snapshots| s.delete(&id)),
                            ),
                    )
            }))
            .children(saved.views.iter().map(|view| {
                let card = col().gap(6.0).padding(12.0).fill(k.c.layer_02);
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
            }));
    } else {
        detail = detail.child(
            text("Select a snapshot to read its saved content.")
                .color(k.c.text_muted)
                .wrap(),
        );
    }
    col()
        .w_full()
        .h_full()
        .padding(24.0)
        .gap(16.0)
        .fill(k.c.bg_base)
        .child(
            text_input(&s.query, k.ty.body_medium)
                .key("snapshot-search")
                .label("Search saved snapshots")
                .single_line()
                .placeholder("Search titles and saved content", k.c.text_muted)
                .color(k.c.text_base)
                .on_edit(|s: &mut Snapshots| &mut s.query)
                .on_input_change(|s: &mut Snapshots, _| {
                    s.filter();
                })
                .on_input_key(Snapshots::key),
        )
        .children(
            s.archive_problem
                .as_deref()
                .or(s.problem.as_deref())
                .map(|p| text(p).color(k.c.text_base).wrap()),
        )
        .child(row().grow(1.0).min_h(0.0).gap(24.0).child(list).child(detail))
}

enum Request {
    Save(Snapshot, crate::ui::Busy),
    Delete(String, crate::ui::Busy),
}

fn publish(
    store: &Store,
    me: &AppHandle<Snapshots>,
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
    me.update(move |s, _| {
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
        s.filter();
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

fn source_name(mode: &str) -> &str {
    match mode {
        "apps.search" => "Applications",
        "files.names" => "Files",
        "files.contents" => "File contents",
        "dictionary.lookup" => "Dictionary",
        "settings.search" => "Settings",
        "web.keywords" | "web.fallbacks" => "Web search",
        _ => mode,
    }
}

fn button(label: &str, k: &rune_kit::Kit) -> El {
    rune_kit::Button::new(label)
        .variant(if label.starts_with("Run again") {
            rune_kit::ButtonVariant::Submit
        } else {
            rune_kit::ButtonVariant::Neutral
        })
        .build(k)
        .key(label)
}

fn saved_date(seconds: u64) -> String {
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
    store: &mut Store,
    rx: std::sync::mpsc::Receiver<Request>,
    mut emit: impl FnMut(&Store, Option<Result<Option<String>, String>>),
) {
    emit(store, None);
    while let Ok(request) = rx.recv() {
        let (result, busy) = match request {
            Request::Save(saved, busy) => (store.save(saved).map(Some), busy),
            Request::Delete(id, busy) => (store.delete(&id).map(|_| None), busy),
        };
        drop(busy);
        emit(store, Some(result));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot_handle() -> AppHandle<Snapshots> {
        rune::App::new(rune::AppSpec::new(
            "snapshots",
            (900.0, 600.0),
            || unreachable!(),
            view,
        ))
        .handle()
    }

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
            me: snapshot_handle(),
            loaded: false,
            sources: vec![],
            entries: vec![saved("other", 2), saved("saved", 1)],
            filtered: vec![0, 1],
            search: vec!["query".into(), "query".into()],
            writer,
            archive_problem: None,
            query: TextEdit::new("query"),
            selected: Some("other".into()),
            pending_delete: None,
            problem: None,
            panel,
            running: Default::default(),
        };
        let mut invoke = |command| {
            let (reply, rx) = std::sync::mpsc::sync_channel(1);
            s.control(
                Call {
                    request: ControlRequest { id: 1, command },
                    cancelled: Default::default(),
                    reply,
                },
                &mut Tx::default(),
            );
            rx.recv().unwrap().result
        };
        assert_eq!(
            invoke(Command::SnapshotsList {}).unwrap_err(),
            "Busy: snapshots loading"
        );
        drop(invoke);
        s.loaded = true;
        for (command, expected) in [
            (
                Command::SnapshotsOpen {
                    id: Some("missing".into()),
                },
                Some("NotFound: snapshot"),
            ),
            (Command::SnapshotsList {}, None),
            (Command::SnapshotsClose {}, None),
        ] {
            let (reply, rx) = std::sync::mpsc::sync_channel(1);
            s.control(
                Call {
                    request: ControlRequest { id: 1, command },
                    cancelled: Default::default(),
                    reply,
                },
                &mut Tx::default(),
            );
            assert_eq!(rx.recv().unwrap().result.err().as_deref(), expected);
            assert_eq!(s.selected.as_deref(), Some("other"));
        }
        let (reply, rx) = std::sync::mpsc::sync_channel(1);
        s.control(
            Call {
                request: ControlRequest {
                    id: 1,
                    command: Command::SnapshotsOpen {
                        id: Some("saved".into()),
                    },
                },
                cancelled: Default::default(),
                reply,
            },
            &mut Tx::default(),
        );
        assert!(
            rx.try_recv().is_err(),
            "opening replies only after the panel's final Busy check"
        );
        assert_eq!(s.selected.as_deref(), Some("saved"));
        assert!(s.query.text.is_empty());
        assert!(received.try_recv().is_err());
    }

    #[test]
    #[ignore = "offscreen screenshot, run explicitly"]
    fn render_saved_content() {
        let panel = rune::App::new(rune::AppSpec::new(
            "panel",
            (1.0, 1.0),
            || unreachable!(),
            crate::view::view,
        ))
        .handle();
        let (writer, received) = std::sync::mpsc::channel();
        let saved = Snapshot {
            version: 1,
            id: "fixture".into(),
            saved_at: 1795392000,
            mode: "apps.search".into(),
            explicit: false,
            query: "2 + 2".into(),
            title: "A saved calculation".into(),
            views: vec![
                dupin_core::View::Pair {
                    source: "2 + 2".into(),
                    result: "4".into(),
                },
                dupin_core::View::Detail {
                    title: "Historical content".into(),
                    body: "Saved results remain readable without reopening an application.".into(),
                },
                dupin_core::View::Text {
                    title: "Serendipity · 偶然の発見".into(),
                    subtitle: "A saved definition".into(),
                },
            ],
            state: None,
        };
        let search = vec![saved.search_text()];
        let state = Snapshots {
            me: snapshot_handle(),
            loaded: true,
            sources: vec!["apps.search".into()],
            entries: vec![saved],
            filtered: vec![0],
            search,
            writer,
            archive_problem: None,
            query: TextEdit::new(""),
            selected: Some("fixture".into()),
            pending_delete: None,
            problem: None,
            panel,
            running: Default::default(),
        };
        let app = rune::App::new(rune::AppSpec::new(
            "Snapshots",
            (900.0, 600.0),
            || unreachable!(),
            view,
        ))
        .init(move || state);
        let script = std::env::temp_dir().join(format!("dupin-snapshot-keys-{}.json", std::process::id()));
        std::fs::write(
            &script,
            r#"{"events":[
            {"t":0.1,"type":"click","key":"snapshot-search"},
            {"t":0.2,"type":"type","text":"2"},
            {"t":0.4,"type":"key","name":"Backspace","cmd":true},
            {"t":0.6,"type":"click","key":"Delete…"},
            {"t":0.8,"type":"click","key":"Cancel"}
        ]}"#,
        )
        .unwrap();
        rune::tooling::run_windows_with_args(
            Default::default(),
            [
                "render",
                "--snap",
                "/tmp/dupin-snapshots-after.png",
                "--theme",
                "dark",
                "--script",
                script.to_str().unwrap(),
            ]
            .map(str::to_owned),
            move |_| [rune::WindowSpec::from_app("snapshots", app)],
        );
        assert!(
            received.try_recv().is_err(),
            "search editing and cancellation must not write or delete"
        );
        std::fs::remove_file(script).unwrap();
    }

    #[test]
    fn writer_reports_success_failure_and_preserves_archive() {
        let dir = std::env::temp_dir().join(format!(
            "dupin-writer-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let (send, rx) = std::sync::mpsc::channel();
        let (reply, results) = std::sync::mpsc::channel();
        let running = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let ready = std::sync::Arc::new(std::sync::Barrier::new(2));
        let worker_ready = ready.clone();
        let path = dir.clone();
        let thread = std::thread::spawn(move || {
            let mut store = Store::load(path);
            write_requests(&mut store, rx, |store, result| {
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
        std::fs::write(dir.join("archive.json"), b"external corruption").unwrap();
        send.send(Request::Save(saved, crate::ui::Busy::new(running.clone())))
            .ok()
            .unwrap();
        assert!(results.recv().unwrap().1.unwrap().is_err());
        assert_eq!(running.load(std::sync::atomic::Ordering::Acquire), 0);
        assert_eq!(
            std::fs::read(dir.join("archive.json")).unwrap(),
            b"external corruption"
        );
        drop(send);
        thread.join().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
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
            me: snapshot_handle(),
            loaded: true,
            sources: vec![],
            entries: vec![],
            filtered: vec![],
            search: vec![],
            writer,
            archive_problem: None,
            query: TextEdit::new("query"),
            selected: Some("saved".into()),
            pending_delete: Some("saved".into()),
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
