use super::runner::{Snapshot, Status};
use rune::{AppHandle, Cx, El, Key, KeyEvent, ScrollSpec, Tx, col, row, text};

pub struct Output {
    pub runs: Vec<Snapshot>,
    pub selected: Option<u64>,
    panel: AppHandle<crate::ui::Ui>,
}
impl Output {
    pub fn new(panel: AppHandle<crate::ui::Ui>) -> Self {
        Self {
            runs: vec![],
            selected: None,
            panel,
        }
    }
    pub fn update(&mut self, runs: Vec<Snapshot>, selected: Option<u64>) {
        self.runs = runs;
        if let Some(id) = selected {
            self.selected = Some(id);
        }
        if !self.runs.iter().any(|run| Some(run.id) == self.selected) {
            self.selected = self.runs.first().map(|run| run.id);
        }
    }
    fn cancel(&mut self) {
        if let Some(id) = self.selected {
            self.panel.update(move |s, _| s.cancel_script(id));
        }
    }
    fn repeat(&mut self) {
        if let Some(run) = self.runs.iter().find(|run| Some(run.id) == self.selected) {
            let command = run.command.clone();
            self.panel.update(move |s, tx| s.repeat_script(command, tx));
        }
    }
    fn copy(&self, tx: &mut Tx) {
        if let Some(run) = self.runs.iter().find(|run| Some(run.id) == self.selected) {
            tx.copy(
                run.output
                    .lines()
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
    }
    pub fn key(&mut self, event: &KeyEvent, tx: &mut Tx) -> bool {
        match event.key {
            Key::Command('.') => self.cancel(),
            Key::Command('r') => self.repeat(),
            Key::Command('c' | 'C') if event.mods.shift => self.copy(tx),
            Key::Escape => tx.close_window(),
            _ => return false,
        }
        true
    }
    pub fn closed(&mut self, _: &mut Tx) {
        self.panel.update(crate::ui::Ui::resume_restart);
    }
}
pub fn view(s: &Output, cx: &mut Cx<Output>) -> El {
    let k = rune_theme_opencode::kit(cx.system_dark());
    let mut list = col()
        .w(220.0)
        .h_full()
        .gap(4.0)
        .scroll(ScrollSpec { stick_bottom: false });
    for run in &s.runs {
        let id = run.id;
        list = list.child(
            col()
                .padding(10.0)
                .gap(4.0)
                .fill(if Some(id) == s.selected {
                    k.c.layer_02
                } else {
                    k.c.bg_base
                })
                .child(text(&run.command.title).font(k.ty.ui_medium).color(k.c.text_base))
                .child(
                    text(format!("{:?}", run.status))
                        .font(k.ty.small)
                        .color(k.c.text_muted),
                )
                .key(format!("script-run-{id}"))
                .on_press(move |s: &mut Output| s.selected = Some(id)),
        );
    }
    let mut detail = col().grow(1.0).min_w(0.0).gap(12.0);
    if let Some(run) = s.runs.iter().find(|run| Some(run.id) == s.selected) {
        detail = detail
            .child(text(&run.command.title).font(k.ty.ui_medium).color(k.c.text_base))
            .child(
                text(format!(
                    "{:?} · {:.1}s · {:?}",
                    run.status,
                    run.elapsed.as_secs_f64(),
                    run.command.args
                ))
                .font(k.ty.small)
                .color(k.c.text_muted)
                .wrap(),
            );
        let button = |label: &str| rune_kit::Button::new(label).build(k).key(label);
        detail = detail.child(
            row()
                .gap(8.0)
                .child(
                    button("Cancel")
                        .disabled(run.status != Status::Running)
                        .on_press(Output::cancel),
                )
                .child(button("Run again").on_press(Output::repeat))
                .child(button("Copy output").on_press_tx(|s: &mut Output, tx| s.copy(tx))),
        );
        let mut lines = col()
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec { stick_bottom: true });
        for (i, line) in run.output.lines().iter().enumerate() {
            let mut style = k.ty.small;
            style.family = rune::tok::Family::SystemMono;
            lines = lines.child(
                text(format!(
                    "{}{}",
                    if line.stream == dupin_core::script::Stream::Stderr {
                        "stderr: "
                    } else {
                        ""
                    },
                    line.text
                ))
                .font(style)
                .color(k.c.text_base)
                .wrap()
                .selectable(true)
                .key(format!("output-{i}")),
            );
        }
        detail = detail.child(lines);
    } else {
        detail = detail.child(text("No script runs yet").color(k.c.text_muted));
    }
    row()
        .h_full()
        .w_full()
        .padding(20.0)
        .gap(20.0)
        .fill(k.c.bg_base)
        .child(list)
        .child(detail)
}
