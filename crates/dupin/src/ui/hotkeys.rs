use dupin_core::config::{Address, AliasTarget};
use dupin_core::script::OutputMode;
use dupin_core::{Action, CommandId, Config, Effect, Item};
use rune::Tx;

use super::Ui;

impl Ui {
    pub fn activate_hotkey(&mut self, shortcut: &rune::Shortcut, tx: &mut Tx) {
        use crate::hotkey::{LauncherShortcut, Target};
        use dupin_core::config::HotkeyTarget;
        crate::trace::hotkey(&shortcut.to_string());

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
            Some(Target::Configured(HotkeyTarget::Dupin { command, query })) => {
                self.reach_bound(shortcut, command, query.as_deref().unwrap_or_default(), tx);
            }
            None => (),
        }
    }

    /// Reaches what a `target = "dupin"` binding addresses: shows the panel in a command or a
    /// web search, or runs a row as choosing it does.
    fn reach_bound(&mut self, shortcut: &rune::Shortcut, address: Address, query: &str, tx: &mut Tx) {
        let keys = shortcut.keycaps().join("");
        match address {
            Address::Alias(AliasTarget::Command(name)) => {
                match crate::features::aliases::command(&name, &self.session.command_ids()) {
                    Ok(command) => self.show_bound(Some(&command), query, tx),
                    Err(problem) => {
                        self.show_notice(format!("Couldn't open {keys}'s command: it {problem}."), tx)
                    }
                }
            }
            Address::Alias(AliasTarget::Web(keyword)) => match self.session.web_search(&keyword) {
                Some(_) => self.show_bound(None, &format!("{keyword} {query}"), tx),
                None => self.show_notice(
                    format!("Couldn't open {keys}'s web search: no web search answers to `{keyword}`."),
                    tx,
                ),
            },
            Address::Alias(AliasTarget::App(bundle_id)) => {
                self.dismiss(tx);
                self.run_workspace_action(dupin_mac::workspace::Action::OpenApp {
                    bundle_id,
                    hide_if_frontmost: false,
                });
            }
            Address::Alias(AliasTarget::File(path)) => {
                self.dismiss(tx);
                let path = crate::features::aliases::expand(&path);
                self.run_workspace_action(dupin_mac::workspace::Action::Open(path));
            }
            Address::Row { command, row } if command == crate::features::system::BINDING_NAME => {
                self.run_bound_system_row(&keys, &row, tx);
            }
            Address::Row { command, row } => self.find_bound_row(keys, &command, row, tx),
        }
    }

    /// Shows the panel in `command`, or the main search without one, with `query` typed; or
    /// hides it when it already shows just that, as the panel's own hotkey toggles it.
    fn show_bound(&mut self, command: Option<&CommandId>, query: &str, tx: &mut Tx) {
        let main = CommandId::new(crate::features::apps::APPS).expect("built-in command IDs are valid");
        let shows = match command {
            Some(command) => self.session.command_id() == command,
            None => self.session.command_id() == &main && self.session.query() == query,
        };
        if self.picker.is_some() {
            self.finish_pick(None, tx);
        } else if self.me.is_visible() && shows {
            self.dismiss(tx);
            return;
        }
        let effects = match command {
            Some(command) => match self.session.enter_command(command, query) {
                Some(effects) => effects,
                None => return,
            },
            None => self.session.search_main(query),
        };
        self.action_screen = None;
        self.apply(effects, tx);
        self.show(tx);
    }

    /// Runs a system command at once; one that asks first shows the panel with its row first,
    /// for ↩ to ask as it always does.
    fn run_bound_system_row(&mut self, keys: &str, row: &str, tx: &mut Tx) {
        let automation = self.configuration.values.borrow().system_automation;
        if let Some(problem) = crate::features::system::unavailable(row, automation, crate::update::offered())
        {
            return self.show_notice(format!("Couldn't run {keys}'s command: it {problem}."), tx);
        }
        let Some(row) = crate::features::system::row(row) else {
            return;
        };
        if row.confirms {
            return self.show_bound(None, row.title, tx);
        }
        self.dismiss(tx);
        self.apply(vec![Effect::Run(row.action)], tx);
    }

    /// Asks the package `package` for its catalogue row `row` off this thread, then runs it.
    fn find_bound_row(&mut self, keys: String, package: &str, row: String, tx: &mut Tx) {
        let Some(extension) = self.registered_extension(package) else {
            return self.show_notice(
                format!(
                    "Couldn't run {keys}'s command: it names `{package}`, which is neither a command nor an extension that is on."
                ),
                tx,
            );
        };
        // The panel isn't up to note the app in front, which the row may act on.
        if !self.shown {
            self.front.capture();
        }
        let bridge = self.extensions.bridge.clone();
        let me = self.me.clone();
        let busy = super::Busy::new(self.running.clone());
        std::thread::spawn(move || {
            let call = bridge.call(&extension.package, dupin_extension_bridge::Trigger::Activation);
            let deadline = std::time::Instant::now() + crate::extensions::QUERY_DEADLINE;
            let found = crate::extensions::catalogue_row(&extension, &row, deadline, &call);
            me.update(move |ui, tx| match found {
                Ok(Some(item)) => ui.run_bound_row(&extension, item, tx),
                Ok(None) => ui.show_notice(
                    format!(
                        "Couldn't run {keys}'s command: {} lists no row `{row}`.",
                        extension.name
                    ),
                    tx,
                ),
                Err(dupin_js_runtime::RunError::Cancelled) => {}
                Err(dupin_js_runtime::RunError::Failed(diagnostic)) => {
                    crate::log::warn(&format!("extension {}: {}", extension.name, diagnostic.for_log()));
                    ui.show_notice(format!("{} failed: {}", extension.name, diagnostic.message), tx);
                }
            });
            drop(busy);
        });
    }

    /// Runs `item`'s primary action as choosing it does. One that asks first, or opens
    /// something to choose from, shows the panel in its package's command with the row's
    /// title typed instead, for ↩ to go on from there.
    fn run_bound_row(&mut self, extension: &crate::extensions::Extension, item: Item, tx: &mut Tx) {
        let action = item.primary.as_deref().and_then(|primary| {
            let confirms = item.item_action(primary)?.confirmation().is_some();
            item.action(primary)
                .filter(|action| runs_unattended(confirms, action))
        });
        let Some(action) = action.cloned() else {
            let command = crate::features::aliases::command(&extension.id, &self.session.command_ids());
            return match command {
                Ok(command) => self.show_bound(Some(&command), item.display_title(), tx),
                Err(problem) => {
                    self.show_notice(format!("Couldn't open {}: it {problem}.", extension.name), tx)
                }
            };
        };
        self.dismiss(tx);
        match action {
            Action::Extension {
                extension,
                request: dupin_core::ExtensionRequest::Run { action, data },
            } => self.run_bound_extension_action(extension, action, data.unwrap_or_default()),
            action => self.apply(vec![Effect::Run(action)], tx),
        }
    }

    /// Whether the command each `target = "dupin"` binding names opens now. One no built-in
    /// answers waits quietly while the extensions are still loading, since one of them may.
    pub(super) fn bound_commands(
        &self,
        config: &Config,
    ) -> impl Fn(&Address) -> Result<(), Option<String>> + use<> {
        let registered = self.session.command_ids();
        let settled = self.extensions.settled();
        let automation = config.system_automation;
        let updates = crate::update::offered();
        let web = self.bound_web_searches(config);
        move |address| {
            let name = match address {
                Address::Alias(AliasTarget::Command(name)) => name,
                Address::Row { command, row } if command == crate::features::system::BINDING_NAME => {
                    return crate::features::system::unavailable(row, automation, updates)
                        .map_or(Ok(()), |problem| Err(Some(problem)));
                }
                // A built-in's rows are what it found, such as clipboard entries, not commands.
                Address::Row { command, .. } if crate::features::aliases::built_in(command).is_some() => {
                    return Err(Some(format!(
                        "names a row of `{command}`, whose rows a binding can't name"
                    )));
                }
                Address::Row { command, .. } => command,
                Address::Alias(AliasTarget::Web(keyword)) if !web.contains(keyword) => {
                    return Err(Some(format!("names the web search `{keyword}`, which isn't one")));
                }
                Address::Alias(_) => return Ok(()),
            };
            match crate::features::aliases::command(name, &registered) {
                Ok(_) => Ok(()),
                Err(_) if !settled && crate::features::aliases::built_in(name).is_none() => Err(None),
                Err(problem) => Err(Some(problem)),
            }
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
        let picker_original_shown = picker.as_ref().map(|_| self.shown);
        if mode == OutputMode::Inline {
            self.action_screen = None;
            self.script_entry = None;
            if !self.shown {
                self.show(tx);
            }
        }
        self.present_script(mode, start, tx);
        match panel_close_after_script(mode, visible, picker_original_shown) {
            Some(PanelClose::Hide) => self.hide(tx),
            Some(PanelClose::Dismiss) => self.dismiss(tx),
            None => {}
        }
        if let Some(picker) = picker {
            picker.finish(Err("Cancelled".into()));
        }
    }

    /// The web searches `config`'s bindings name that a web search answers now.
    pub(super) fn bound_web_searches(&self, config: &Config) -> Vec<String> {
        config
            .bindings
            .values()
            .filter_map(|target| match target {
                dupin_core::config::HotkeyTarget::Dupin {
                    command: Address::Alias(AliasTarget::Web(keyword)),
                    ..
                } => self.session.web_search(keyword).map(|_| keyword.clone()),
                _ => None,
            })
            .collect()
    }

    pub(super) fn apply_hotkeys(&mut self, config: &Config, previous: &Config) -> Result<(), String> {
        let commands = self.bound_commands(config);
        let problem = {
            let scripts = self.scripts.read().unwrap();
            self.configuration.hotkey.apply_config(
                config,
                previous,
                self.scripts_catalog_ready.then_some(&*scripts),
                &commands,
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

/// Whether choosing `action` needs nothing more of the user than the press of a shortcut: no
/// confirmation, no list to choose from and no page to read.
fn runs_unattended(confirms: bool, action: &Action) -> bool {
    !confirms
        && !matches!(
            action,
            Action::Push(_)
                | Action::Trash(_)
                | Action::Browse(_)
                | Action::Keyword(_)
                | Action::Rename(_)
                | Action::ChooseApplication(_)
                | Action::ChooseDestination { .. }
                | Action::ScriptArguments(_)
        )
}

#[derive(Debug, PartialEq, Eq)]
enum PanelClose {
    Hide,
    Dismiss,
}

fn panel_close_after_script(
    mode: OutputMode,
    visible: bool,
    picker_original_shown: Option<bool>,
) -> Option<PanelClose> {
    if mode == OutputMode::Inline || !visible {
        return None;
    }
    Some(match picker_original_shown {
        Some(false) => PanelClose::Hide,
        _ => PanelClose::Dismiss,
    })
}

#[test]
fn non_inline_script_closes_a_picker_opened_over_a_hidden_panel() {
    for mode in [OutputMode::Full, OutputMode::Silent] {
        assert_eq!(
            panel_close_after_script(mode, true, Some(false)),
            Some(PanelClose::Hide)
        );
        assert_eq!(
            panel_close_after_script(mode, true, Some(true)),
            Some(PanelClose::Dismiss)
        );
        assert_eq!(
            panel_close_after_script(mode, true, None),
            Some(PanelClose::Dismiss)
        );
        assert_eq!(panel_close_after_script(mode, false, None), None);
    }
    assert_eq!(
        panel_close_after_script(OutputMode::Inline, true, Some(false)),
        None
    );
}

#[test]
fn a_bound_row_runs_unattended_unless_it_asks_or_opens_something() {
    let run = Action::Extension {
        extension: "window-management".into(),
        request: dupin_core::ExtensionRequest::Run {
            action: "arrange".into(),
            data: Some("left-half".into()),
        },
    };
    assert!(runs_unattended(false, &run));
    assert!(runs_unattended(
        false,
        &Action::OpenUrl("https://example.com".into())
    ));
    assert!(!runs_unattended(true, &run));
    assert!(!runs_unattended(false, &Action::Trash("/tmp/x".into())));
    assert!(!runs_unattended(
        false,
        &Action::Push(dupin_core::Page {
            view: "details".into(),
            title: "Details".into(),
            context: None,
        })
    ));
}
