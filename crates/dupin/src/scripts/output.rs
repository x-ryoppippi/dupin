use super::runner::{RunId, Snapshot, Status};
use rune::{AppHandle, Cx, El, Key, KeyEvent, ScrollSpec, Tx, col, row, text};

pub struct Output {
    runs: Vec<Snapshot>,
    selected: Option<RunId>,
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
    pub fn update(&mut self, runs: Vec<Snapshot>, selected: Option<RunId>) {
        self.runs = runs;
        if let Some(id) = selected {
            self.selected = Some(id);
        }
        if self.selected_run().is_none() {
            self.selected = self.runs.first().map(|run| run.id);
        }
    }
    fn selected_run(&self) -> Option<&Snapshot> {
        self.runs.iter().find(|run| Some(run.id) == self.selected)
    }
    fn cancel(&mut self) {
        if let Some(id) = self.selected {
            self.panel.update(move |s, _| s.cancel_script(id));
        }
    }
    fn repeat(&mut self) {
        if let Some(run) = self.selected_run() {
            let command = run.command.clone();
            self.panel.update(move |s, tx| s.start_script(command, tx));
        }
    }
    fn copy(&self, tx: &mut Tx) {
        if let Some(run) = self.selected_run() {
            let notice = dropped_notice(&run.output);
            tx.copy(
                notice
                    .as_deref()
                    .into_iter()
                    .chain(run.output.lines().iter().map(|line| line.text.as_str()))
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

fn dropped_notice(output: &dupin_core::script::OutputBuffer) -> Option<String> {
    (output.dropped() > 0).then(|| format!("… {} lines dropped", output.dropped()))
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
                .w_full()
                .px(10.0)
                .py(8.0)
                .gap(2.0)
                .radius(8.0)
                .fill(if Some(id) == s.selected {
                    k.c.layer_02
                } else {
                    k.c.bg_base
                })
                .hover(k.c.layer_02)
                .child(
                    text(&run.command.title)
                        .font(k.ty.ui_medium)
                        .color(k.c.text_base)
                        .truncate(),
                )
                .child(
                    text(run.status.to_string())
                        .font(k.ty.small)
                        .color(k.c.text_muted)
                        .truncate(),
                )
                .focusable(true)
                .key(format!("script-run-{id}"))
                .on_press(move |s: &mut Output| s.selected = Some(id)),
        );
    }
    let mut detail = col().grow(1.0).min_w(0.0).gap(12.0);
    if let Some(run) = s.selected_run() {
        detail = detail.child(
            col()
                .w_full()
                .gap(4.0)
                .child(
                    text(&run.command.title)
                        .w_full()
                        .font(k.ty.body_medium)
                        .color(k.c.text_base)
                        .truncate(),
                )
                .child(
                    text(run.summary())
                        .w_full()
                        .font(k.ty.small)
                        .color(if run.status.failed() {
                            k.c.text_base
                        } else {
                            k.c.text_muted
                        })
                        .wrap(),
                ),
        );
        if !run.command.args.is_empty() {
            detail =
                detail.child(
                    row()
                        .w_full()
                        .gap(12.0)
                        .child(text("Arguments").font(k.ty.small).color(k.c.text_muted))
                        .child(
                            col()
                                .grow(1.0)
                                .min_w(0.0)
                                .shrink()
                                .max_h(72.0)
                                .scroll(ScrollSpec { stick_bottom: false })
                                .child(col().w_full().gap(2.0).children(run.command.args.iter().map(
                                    |value| {
                                        let (value, color) = if value.is_empty() {
                                            ("Empty string", k.c.text_muted)
                                        } else {
                                            (value.as_str(), k.c.text_base)
                                        };
                                        text(value).w_full().font(k.ty.small).color(color).wrap()
                                    },
                                ))),
                        ),
                );
        }
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
        if let Some(notice) = dropped_notice(&run.output) {
            detail = detail.child(
                text(notice)
                    .font(k.ty.small)
                    .color(k.c.text_muted)
                    .key("script-output-dropped"),
            );
        }
        let mut lines = col().w_full().padding(12.0).gap(2.0).selection_group(true);
        let mut style = k.ty.ui;
        style.family = rune::tok::Family::SystemMono;
        for (i, line) in run.output.lines().iter().enumerate() {
            let stderr = line.stream == dupin_core::script::Stream::Stderr;
            lines = lines.child(
                text(format!("{}{}", if stderr { "stderr: " } else { "" }, line.text))
                    .w_full()
                    .font(style)
                    .color(if stderr { k.c.danger.fg } else { k.c.text_base })
                    .wrap()
                    .selectable(true)
                    .key(format!("output-{i}")),
            );
        }
        detail = detail.child(
            col()
                .key(format!("script-output-{}", run.id))
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .radius(8.0)
                .fill(k.c.layer_01)
                .scroll(ScrollSpec { stick_bottom: true })
                .child(lines),
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "offscreen screenshot, run explicitly"]
    fn render_output_fixture() {
        let panel = rune::App::new(rune::AppSpec::new(
            "panel",
            (1.0, 1.0),
            || unreachable!(),
            crate::view::view,
        ))
        .handle();
        let mut state = Output::new(panel);
        let mut output = dupin_core::script::OutputBuffer::default();
        output.push(dupin_core::script::Stream::Stdout, b"Stream line 1");
        output.push(dupin_core::script::Stream::Stdout, b"Stream line 2");
        output.push(dupin_core::script::Stream::Stderr, b"A fixture warning");
        state.update(
            vec![Snapshot {
                id: 1,
                command: dupin_core::script::ScriptRun {
                    path: "/fixture/stream".into(),
                    title: "Fixture stream".into(),
                    args: vec!["hello world".into()],
                    mode: dupin_core::script::OutputMode::Full,
                },
                status: Status::Running,
                output,
                elapsed: std::time::Duration::from_secs(2),
            }],
            Some(1),
        );
        let app = rune::App::new(rune::AppSpec::new(
            "Dupin Script Output",
            (900.0, 600.0),
            || unreachable!(),
            view,
        ))
        .init(move || state);
        let path = std::env::temp_dir().join("dupin35-output.png");
        rune::tooling::run_windows_with_args(
            Default::default(),
            [
                "render",
                "--window",
                "scripts",
                "--snap",
                path.to_str().unwrap(),
                "--theme",
                "dark",
            ]
            .map(str::to_owned),
            move |_| [rune::WindowSpec::from_app("scripts", app)],
        );
        assert!(path.exists());
    }
}
