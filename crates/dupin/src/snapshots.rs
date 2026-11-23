use dupin_core::snapshots::{Snapshot, Store};
use rune::{AppHandle, Cx, El, Key, KeyEvent, ScrollSpec, TextEdit, Tx, col, row, text, text_input};

pub struct Snapshots {
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
}

impl Snapshots {
    pub fn new(panel: AppHandle<crate::ui::Ui>, me: AppHandle<Self>) -> Self {
        let (writer, rx) = std::sync::mpsc::channel();
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
        }
    }

    pub fn set_sources(&mut self, sources: Vec<String>) {
        self.sources = sources;
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

    pub fn save(&mut self, snapshot: Snapshot) {
        if self.writer.send(Request::Save(snapshot)).is_err() {
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
        let _ = self.writer.send(Request::Delete(id.into()));
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
                        button("Run again (⌘R)", &k)
                            .disabled(!s.sources.contains(&saved.mode))
                            .color(k.c.text_base)
                            .focusable(true)
                            .on_press(|s: &mut Snapshots| s.resume()),
                    )
                    .child(
                        button("Delete…", &k)
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
                                button("Cancel", &k)
                                    .focusable(true)
                                    .color(k.c.text_base)
                                    .on_press(|s: &mut Snapshots| s.pending_delete = None),
                            )
                            .child(
                                button("Delete snapshot", &k)
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
    Save(Snapshot),
    Delete(String),
}

fn publish(
    store: &Store,
    me: &AppHandle<Snapshots>,
    result: Option<Result<Option<String>, String>>,
    cache: &mut std::collections::HashMap<String, String>,
) {
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
        let result = match request {
            Request::Save(saved) => store.save(saved).map(Some),
            Request::Delete(id) => store.delete(&id).map(|_| None),
        };
        emit(store, Some(result));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let path = dir.clone();
        let thread = std::thread::spawn(move || {
            let mut store = Store::load(path);
            write_requests(&mut store, rx, |store, result| {
                reply.send((store.list().to_vec(), result)).unwrap();
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
        send.send(Request::Save(saved.clone())).unwrap();
        let (entries, result) = results.recv().unwrap();
        let id = result.unwrap().unwrap().unwrap();
        assert_eq!(entries[0].id, id);
        assert!(entries[0].saved_at > 0);
        std::fs::write(dir.join("archive.json"), b"external corruption").unwrap();
        send.send(Request::Save(saved)).unwrap();
        assert!(results.recv().unwrap().1.unwrap().is_err());
        assert_eq!(
            std::fs::read(dir.join("archive.json")).unwrap(),
            b"external corruption"
        );
        drop(send);
        thread.join().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
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
