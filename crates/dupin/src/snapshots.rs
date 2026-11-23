use dupin_core::snapshots::{Snapshot, Store};
use rune::{AppHandle, Cx, El, Key, KeyEvent, ScrollSpec, TextEdit, Tx, col, row, text, text_input};

pub struct Snapshots {
    store: Store,
    query: TextEdit,
    selected: Option<String>,
    pending_delete: Option<String>,
    problem: Option<String>,
    panel: AppHandle<crate::ui::Ui>,
}

impl Snapshots {
    pub fn new(panel: AppHandle<crate::ui::Ui>) -> Self {
        let directory = std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .map(|p| p.join("dupin/snapshots"))
            .unwrap_or_else(|| {
                std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                    .join("Library/Application Support/Dupin/snapshots")
            });
        let mut query = TextEdit::new("");
        query.focused = true;
        Self {
            store: Store::load(directory),
            query,
            selected: None,
            pending_delete: None,
            problem: None,
            panel,
        }
    }

    pub fn save(&mut self, snapshot: Snapshot) {
        let message = match self.store.save(snapshot) {
            Ok(id) => {
                self.selected = Some(id);
                "Snapshot saved. Open Snapshots with ⌘⇧O.".into()
            }
            Err(problem) => problem,
        };
        self.problem = Some(message.clone());
        self.panel.update(move |ui, _| ui.notice = Some(message));
    }

    pub fn open(&mut self, id: &str) -> bool {
        if self.store.open(id).is_none() {
            return false;
        }
        self.selected = Some(id.into());
        self.pending_delete = None;
        true
    }

    pub fn delete(&mut self, id: &str) {
        self.pending_delete = None;
        match self.store.delete(id) {
            Ok(()) => {
                self.selected = None;
                self.problem = None;
            }
            Err(problem) => self.problem = Some(problem),
        }
    }

    fn resume(&mut self) {
        if let Some(saved) = self
            .selected
            .as_deref()
            .and_then(|id| self.store.open(id))
            .cloned()
        {
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
            Key::Escape => tx.close_window(),
            Key::Command('r') => self.resume(),
            Key::Command('w') => tx.close_window(),
            Key::Down | Key::Up => {
                let entries: Vec<_> = self
                    .store
                    .list()
                    .iter()
                    .filter(|s| s.matches(&self.query.text))
                    .collect();
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
    for saved in s.store.list().iter().filter(|s2| s2.matches(&s.query.text)) {
        count += 1;
        let id = saved.id.clone();
        let selected = s.selected.as_ref() == Some(&id);
        list = list.child(
            text(&saved.title)
                .key(format!("snapshot-{id}"))
                .label(&saved.title)
                .font(k.ty.body_medium)
                .color(k.c.text_base)
                .wrap()
                .padding(10.0)
                .focusable(true)
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
            text(if s.store.problem().is_some() {
                "Saved snapshots are unavailable until the archive problem is resolved."
            } else {
                "No snapshots found. Save a settled search with ⌘⇧S."
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
    if let Some(saved) = s.selected.as_deref().and_then(|id| s.store.open(id)) {
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
                    .font(k.ty.body_medium)
                    .color(k.c.text_base)
                    .wrap(),
            )
            .child(
                text(format!("{} · {}", saved.mode, saved.query))
                    .font(k.ty.small)
                    .color(k.c.text_muted)
                    .wrap(),
            )
            .child(
                row()
                    .gap(20.0)
                    .child(
                        text("Resume search (⌘R)")
                            .color(k.c.text_base)
                            .focusable(true)
                            .on_press(|s: &mut Snapshots| s.resume()),
                    )
                    .child(
                        text("Delete…")
                            .color(k.c.text_base)
                            .focusable(true)
                            .on_press(move |s: &mut Snapshots| s.pending_delete = Some(delete_id.clone())),
                    ),
            )
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
                                text("Cancel")
                                    .focusable(true)
                                    .color(k.c.text_base)
                                    .on_press(|s: &mut Snapshots| s.pending_delete = None),
                            )
                            .child(
                                text("Delete snapshot")
                                    .focusable(true)
                                    .color(k.c.text_base)
                                    .on_press(move |s: &mut Snapshots| s.delete(&id)),
                            ),
                    )
            }))
            .child(
                text(saved.content())
                    .font(k.ty.body_medium)
                    .color(k.c.text_base)
                    .wrap(),
            );
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
            text("Snapshots")
                .font(k.ty.body_medium)
                .color(k.c.text_base)
                .window_drag(),
        )
        .child(
            text_input(&s.query, k.ty.body_medium)
                .key("snapshot-search")
                .label("Search saved snapshots")
                .single_line()
                .placeholder("Search titles and saved content", k.c.text_muted)
                .color(k.c.text_base)
                .on_edit(|s: &mut Snapshots| &mut s.query)
                .on_input_change(|s: &mut Snapshots, _| {
                    if s.selected
                        .as_deref()
                        .and_then(|id| s.store.open(id))
                        .is_some_and(|saved| !saved.matches(&s.query.text))
                    {
                        s.selected = None;
                    }
                })
                .on_input_key(Snapshots::key),
        )
        .children(
            s.store
                .problem()
                .or(s.problem.as_deref())
                .map(|p| text(p).color(k.c.text_base).wrap()),
        )
        .child(row().grow(1.0).min_h(0.0).gap(24.0).child(list).child(detail))
}
