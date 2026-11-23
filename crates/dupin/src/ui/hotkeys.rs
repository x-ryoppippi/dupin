use dupin_core::Config;
use rune::Tx;

use super::Ui;

impl Ui {
    pub fn activate_hotkey(&mut self, shortcut: &rune::Shortcut, tx: &mut Tx) {
        use crate::hotkey::{LauncherShortcut, Target};
        use dupin_core::config::HotkeyTarget;

        match self.configuration.hotkey.target(shortcut) {
            Some(Target::Launcher(LauncherShortcut::Panel)) => self.toggle(tx),
            Some(Target::Launcher(LauncherShortcut::Selection)) => self.read_selection(tx),
            Some(Target::Configured(HotkeyTarget::App {
                bundle_id,
                hide_if_frontmost,
            })) => {
                if self.shown {
                    self.dismiss(tx);
                }
                self.run_workspace_action(dupin_mac::workspace::Action::OpenApp {
                    bundle_id,
                    hide_if_frontmost,
                });
            }
            Some(Target::Configured(HotkeyTarget::Command { command })) => {
                self.run_hotkey_command(shortcut, command, tx);
            }
            Some(Target::Configured(HotkeyTarget::Script { script, args })) => {
                self.run_hotkey_script(&script, args, tx);
            }
            None => (),
        }
    }

    fn run_hotkey_command(&mut self, shortcut: &rune::Shortcut, command: String, tx: &mut Tx) {
        let panel = self.me.clone();
        let keys = shortcut.keycaps().join("");
        if let Err(error) = self.runner.start_shell(&command, move |result| {
            if let Err(error) = result {
                panel.update(move |ui, tx| {
                    ui.show_notice(format!("The {keys} command failed: {error}"), tx);
                });
            }
        }) {
            self.show_notice(
                format!(
                    "Couldn't start the {} command: {error}",
                    shortcut.keycaps().join("")
                ),
                tx,
            );
        }
    }

    fn run_hotkey_script(&mut self, name: &str, args: Vec<String>, tx: &mut Tx) {
        use dupin_core::script::OutputMode;

        let command = self
            .scripts
            .read()
            .unwrap()
            .bound_script(name, &args)
            .map(|script| script.run(args));
        let command = match command {
            Ok(command) => command,
            Err(problem) => {
                self.show_notice(format!("Couldn't run {name:?}. {problem}."), tx);
                return;
            }
        };
        let mode = command.mode;
        let start = self.runner.start(command);
        let visible = self.me.is_visible();
        let picker = self.restore_picker();
        if mode == OutputMode::Inline {
            self.action_screen = None;
            self.script_entry = None;
            if !self.shown {
                self.show(tx);
            }
        }
        self.present_script(mode, start, tx);
        if mode != OutputMode::Inline && visible {
            self.dismiss(tx);
        }
        if let Some(picker) = picker {
            picker.finish(Err("Cancelled".into()));
        }
    }

    pub(super) fn apply_hotkeys(&mut self, config: &Config, previous: &Config) -> Result<(), String> {
        let problem = {
            let scripts = self.scripts.read().unwrap();
            self.configuration.hotkey.apply_config(
                config,
                previous,
                self.scripts_catalog_ready.then_some(&*scripts),
            )?
        };
        self.bindings_applied = true;
        if self.hotkey_problem != problem {
            if let Some(problem) = &problem {
                crate::log::warn(problem);
            }
            self.hotkey_problem = problem.clone();
            self.prefs.update(move |prefs, _| prefs.hotkey_problem = problem);
        }
        Ok(())
    }

    pub(super) fn refresh_hotkeys(&mut self) {
        if self.bindings_applied {
            let config = self.configuration.values.borrow().clone();
            if let Err(error) = self.apply_hotkeys(&config, &config) {
                crate::log::warn(&error);
            }
        }
    }
}
