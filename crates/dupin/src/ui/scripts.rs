use std::path::PathBuf;
use std::sync::Arc;

use rune::{Key, KeyEvent, TextEdit, Tx};

use crate::scripts::catalog::Catalog;

use super::Ui;

impl Ui {
    pub(crate) fn scripts_catalog(&mut self, mut catalog: Catalog, tx: &mut Tx) {
        let notice = catalog.notice.take();
        self.prefs.update(move |prefs, _| prefs.scripts_notice = notice);
        let changed = {
            let mut current = self.scripts.write().unwrap();
            let changed = current.scripts != catalog.scripts;
            if changed {
                *current = catalog;
            }
            changed
        };
        let first = !std::mem::replace(&mut self.scripts_catalog_ready, true);
        // The save acknowledgement applies the latest catalogue without releasing reserved chords early.
        if (first || changed) && self.control_save.is_none() {
            self.refresh_hotkeys();
        }
        if changed {
            self.refresh_default(tx);
        }
    }

    pub(super) fn script_entry_key(&mut self, ev: &KeyEvent, tx: &mut Tx) -> bool {
        let Some(entry) = &self.script_entry else {
            return false;
        };
        if entry.query.is_composing() {
            return false;
        }
        if matches!(ev.key, Key::Escape) || (matches!(ev.key, Key::Backspace) && entry.query.text.is_empty())
        {
            self.script_entry = None;
            tx.focus("query", false);
            return true;
        }
        if matches!(ev.key, Key::Enter) {
            let effects = crate::scripts::entry::Entry::submit(
                &mut self.script_entry,
                &mut self.session,
                &self.scripts.read().unwrap(),
            );
            self.apply(effects, tx);
            return true;
        }
        false
    }

    pub(crate) fn script_entry_current(&self, entry: &crate::scripts::entry::Entry) -> bool {
        entry.is_current(&self.session, &self.scripts.read().unwrap())
    }

    pub(crate) fn cancel_script(&mut self, id: crate::scripts::runner::RunId) {
        self.runner.cancel(id);
    }

    pub(super) fn show_script_output(&self, id: crate::scripts::runner::RunId) {
        let runs = self.runner.snapshots();
        self.script_output.update(move |s, _| s.update(runs, Some(id)));
        self.script_output.open();
    }

    pub(super) fn script_changed(
        &mut self,
        id: crate::scripts::runner::RunId,
        finished: bool,
        tx: &mut Tx,
    ) -> bool {
        let Some(snapshot) = self.runner.snapshot(id) else {
            return false;
        };
        if !finished && snapshot.status != crate::scripts::runner::Status::Running {
            return false;
        }
        let changed = self
            .script_card
            .as_ref()
            .is_some_and(|card| card.id == id && !Arc::ptr_eq(card, &snapshot));
        if changed {
            self.script_card = Some(snapshot.clone());
            tx.instant();
        }
        let inline_card = self
            .script_card
            .as_ref()
            .filter(|_| self.picker.is_none())
            .map(|card| card.id);
        if crate::scripts::output::show_failure(&snapshot, inline_card, self.me.is_visible()) {
            self.show_script_output(id);
        } else {
            self.script_output.update_if(move |s, _| s.replace_run(snapshot));
        }
        changed
    }

    fn launch_script(&self, command: dupin_core::script::ScriptRun) -> crate::scripts::runner::Start {
        let valid = self
            .scripts
            .read()
            .unwrap()
            .scripts
            .get(&command.path)
            .is_some_and(|s| {
                s.header.title == command.title
                    && s.header.mode == command.mode
                    && s.header.bind_args(&command.args).is_ok()
            });
        if valid {
            self.runner.start(command)
        } else {
            self.runner
                .refuse(command, "Script changed. Search again before running it.".into())
        }
    }

    pub(crate) fn repeat_script(&self, command: dupin_core::script::ScriptRun) {
        let start = self.launch_script(command);
        self.show_script_output(start.id);
    }

    /// Run the script a kind's configured action names on the row's value. The panel stays for
    /// an inline card and closes otherwise, as when the script is run from its own row.
    pub(super) fn run_configured_script(&mut self, name: &str, argument: String, tx: &mut Tx) {
        let args = vec![argument];
        let command = self
            .scripts
            .read()
            .unwrap()
            .bound_script(name, &args)
            .map(|script| script.run(args));
        match command {
            Ok(command) => {
                let inline = command.mode == dupin_core::script::OutputMode::Inline;
                self.start_script(command, tx);
                if !inline {
                    self.dismiss(tx);
                }
            }
            Err(problem) => self.show_notice(format!("Couldn't run {name:?}. {problem}."), tx),
        }
    }

    pub(super) fn start_script(&mut self, command: dupin_core::script::ScriptRun, tx: &mut Tx) {
        let mode = command.mode;
        let start = self.launch_script(command);
        self.present_script(mode, start, tx);
    }

    pub(super) fn present_script(
        &mut self,
        mode: dupin_core::script::OutputMode,
        start: crate::scripts::runner::Start,
        tx: &mut Tx,
    ) {
        use dupin_core::script::OutputMode;
        match mode {
            OutputMode::Inline => {
                self.script_card = self.runner.snapshot(start.id);
                tx.focus("query", false);
            }
            OutputMode::Silent if !start.refused => {}
            OutputMode::Full | OutputMode::Silent => {
                if self.me.is_visible() {
                    self.script_output_after_close = Some(start.id);
                } else {
                    self.show_script_output(start.id);
                    return;
                }
            }
        }
        if self.script_output_after_close.is_none() && self.script_output.is_visible() {
            let runs = self.runner.snapshots();
            self.script_output.update(move |s, _| s.update(runs, None));
        }
    }

    pub(super) fn open_script_arguments(&mut self, path: PathBuf, tx: &mut Tx) {
        let script = self.scripts.read().unwrap().scripts.get(&path).cloned();
        if let Some(script) = script
            && self
                .session
                .selected_item()
                .and_then(|item| item.action(crate::scripts::provider::ARGUMENTS))
                == Some(&dupin_core::Action::ScriptArguments(path))
            && let Some(target) = self.session.followup_target(crate::scripts::provider::ARGUMENTS)
        {
            let mut query = TextEdit::new("");
            query.focused = true;
            self.script_card = None;
            self.script_entry = Some(crate::scripts::entry::Entry {
                target,
                script,
                query,
            });
            tx.focus("script-arguments", false);
        }
    }

    pub(super) fn script_card_key(&mut self, ev: &KeyEvent, tx: &mut Tx) -> bool {
        if let Some(card) = &self.script_card {
            if is_script_output_shortcut(ev) {
                self.show_script_output(card.id);
                return true;
            }
            if matches!(ev.key, Key::Enter) {
                if card.status != crate::scripts::runner::Status::Exited(Some(0)) {
                    return true;
                }
                if let Some(text) = card.output.last_stdout() {
                    tx.copy(text.to_owned());
                }
                self.dismiss(tx);
                return true;
            }
            if ev.mods.cmd && matches!(ev.key, Key::Command('.')) {
                self.cancel_script(card.id);
                return true;
            }
        }
        false
    }
}

fn is_script_output_shortcut(ev: &KeyEvent) -> bool {
    ev.mods.cmd && !ev.mods.shift && matches!(ev.key, Key::Command('o'))
}

#[test]
fn script_output_shortcut_does_not_consume_the_snapshots_chord() {
    let mut ev = KeyEvent {
        key: Key::Command('o'),
        mods: rune::Mods {
            cmd: true,
            ..Default::default()
        },
    };
    assert!(is_script_output_shortcut(&ev));
    ev.mods.shift = true;
    assert!(!is_script_output_shortcut(&ev));
}
