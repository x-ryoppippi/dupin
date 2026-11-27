use dupin_core::{Action, CommandId, Effect, Session};
use rune::Tx;
use rune::raw_window_handle::RawWindowHandle;

use super::{Busy, Ui, query::mark_session_query};

impl Ui {
    pub(super) fn apply(&mut self, effects: Vec<Effect>, tx: &mut Tx) {
        // Rune animates a key's commit but lands typing and what no key in the panel caused (a
        // hotkey, an extension's answer, a search's results) at once; entering or leaving a
        // command moves the same way whatever caused it, up to its first results.
        if entered(&mut self.entered, &self.session) {
            self.entering = self.on_screen();
            if self.entering {
                self.entered_at = tx.clock();
                tx.animated();
            }
        }
        for effect in effects {
            match effect {
                Effect::ShowActions { id, confirmation, .. } => self.show_action_screen(id, confirmation),
                Effect::Search {
                    command,
                    generation,
                    query,
                    folder,
                    page,
                } => self.start_search(command, generation, query, folder, page, tx),
                Effect::CancelSearch => self.cancel_search(),
                Effect::ClearQuery => {
                    self.script_card = None;
                    self.script_entry = None;
                    self.action_screen = None;
                    if let Some(picker) = &mut self.picker {
                        picker.original.clear();
                    } else {
                        self.query.clear();
                    }
                }
                Effect::ReplaceQuery(text) => {
                    self.script_card = None;
                    self.script_entry = None;
                    self.query.replace_text(text);
                }
                Effect::SelectQuery(range) => {
                    tx.focus("query", false);
                    self.query.set_selection(range.start, range.end);
                }
                Effect::SelectRenameStem(path) => self.select_rename_stem(&path, tx),
                Effect::Rename { row, path, name } => self.rename_file(row, path, name),
                Effect::Transfer {
                    transfer,
                    path,
                    folder,
                } => self.transfer_file(transfer, path, folder),
                Effect::SaveHistory => self.sync.save_usage(self.session.own_history()),
                Effect::Run(action) => match action {
                    Action::RunScript(command) => self.start_script(command, tx),
                    Action::ConfiguredScript { script, argument } => {
                        self.run_configured_script(&script, argument, tx)
                    }
                    Action::ScriptArguments(path) => self.open_script_arguments(path, tx),
                    Action::Copy(text) => tx.copy(text),
                    Action::Snippet { text, paste } => self.run_snippet(text, paste, tx),
                    Action::Share { target, paths } => {
                        if !rune::tooling::is_headless()
                            && let Err(error) = self.share(target, &paths)
                        {
                            self.me.update(move |ui, tx| ui.show_notice(error, tx));
                        }
                    }
                    Action::OpenPreferences => self.prepare_settings(),
                    Action::OpenSnapshots(id) => self.prepare_snapshots(id),
                    Action::ResumeSnapshot(id) => {
                        self.prefs.update(move |prefs, _| prefs.snapshots.resume_id(&id))
                    }
                    Action::App(command) => self.run_app_command(command, tx),
                    Action::System(command) => {
                        let run = self.system_runner;
                        let enabled = self.configuration.values.borrow().system_automation;
                        self.run_native_action(move || {
                            dupin_mac::system_commands::execute(command, enabled, run)
                        });
                    }
                    Action::Preview(path) => {
                        let shown = self
                            .quick_look
                            .borrow()
                            .as_ref()
                            .is_some_and(|preview| preview.show(&path));
                        if !shown {
                            crate::log::warn("couldn't show the Quick Look preview");
                        }
                    }
                    Action::CopyFiles(paths) => self.copy_files(&paths, tx),

                    Action::ChooseApplication(_)
                    | Action::Trash(_)
                    | Action::Browse(_)
                    | Action::Push(_)
                    | Action::Keyword(_)
                    | Action::Rename(_)
                    | Action::ChooseDestination { .. } => {}
                    Action::RequestClipboardAccess => self.request_clipboard_access(),
                    Action::Extension { extension, request } => {
                        self.run_extension_request(extension, request, tx)
                    }
                    Action::Duplicate(path) => self.duplicate_file(path),
                    Action::Compress(path) => self.compress_file(path),
                    Action::OpenWith { path, application } => self
                        .run_workspace_action(dupin_mac::workspace::Action::OpenWith { path, application }),
                    Action::Open(path) => self.run_workspace_action(dupin_mac::workspace::Action::Open(path)),
                    Action::Reveal(path) => {
                        self.run_workspace_action(dupin_mac::workspace::Action::Reveal(path))
                    }
                    Action::OpenUrl(url) => {
                        self.run_workspace_action(dupin_mac::workspace::Action::OpenUrl(url))
                    }
                },
                Effect::RunConsuming {
                    id,
                    action: Action::Trash(path),
                } => self.trash_file(id, path),

                Effect::RunConsuming {
                    id,
                    action: Action::Extension { extension, request },
                } => self.answer_extension_consent(id, extension, request, tx),
                Effect::RunConsuming { .. } => crate::log::warn("unsupported consuming action"),
                Effect::Hide => self.hide(tx),
                Effect::RevealSelection => self.reveal_selection(tx),
            }
        }
        self.sync_forms(tx);
        // The selected row may have changed, and with it the preview its detail shows.
        self.fetch_icons();
        mark_session_query(&mut self.query, &self.session);
    }

    pub(super) fn reveal_selection(&self, tx: &mut Tx) {
        if self.session.selected_item().is_some() {
            tx.scroll_into_view(crate::view::row_key(
                &self.session.layout_key(self.session.selected()),
            ));
        }
    }

    pub(super) fn run_workspace_action(&self, action: dupin_mac::workspace::Action) {
        self.run_native_action(move || dupin_mac::workspace::run(&action));
    }

    fn run_native_action(&self, run: impl FnOnce() -> Result<(), String> + Send + 'static) {
        let me = self.me.clone();
        let busy = Busy::new(self.running.clone());
        std::thread::spawn(move || {
            if let Err(error) = run() {
                me.update(move |ui, tx| ui.show_notice(error, tx));
            }
            drop(busy);
        });
    }

    fn share(&mut self, target: dupin_core::ShareTarget, paths: &[std::path::PathBuf]) -> Result<(), String> {
        let window = self
            .panel_window
            .as_ref()
            .ok_or("Couldn't find the launcher window.")?;
        let panel = self.me.clone();
        window.with_raw_handle(|handle| match handle.as_raw() {
            // SAFETY: Rune lends the live launcher view for this main-thread callback.
            RawWindowHandle::AppKit(appkit) => unsafe {
                self.sharing.share(target, paths, appkit.ns_view, move || {
                    panel.update(|ui, tx| ui.dismiss_if_unfocused(tx));
                })
            },
            _ => Err("Couldn't find the launcher view.".into()),
        })?
    }
}

/// Whether the session shows another command than `shown`, which it then becomes.
fn entered(shown: &mut CommandId, session: &Session) -> bool {
    if session.command_id() == shown {
        return false;
    }
    shown.clone_from(session.command_id());
    true
}

#[cfg(test)]
mod tests {
    use dupin_core::searcher::Delivery;
    use dupin_core::{Config, History, Intent, Item};

    use super::*;

    fn session() -> Session {
        let config = Config::parse("[calendar]\nenabled = true").unwrap();
        Session::new(
            History::default(),
            crate::commands::configured(&config).unwrap().registry,
        )
        .unwrap()
    }

    /// The main search for `query`, answered by one app.
    fn typed(session: &mut Session, query: &str) {
        let generation = session
            .set_query(query)
            .iter()
            .find_map(|effect| match effect {
                Effect::Search {
                    command, generation, ..
                } if command.as_str() == crate::features::apps::APPS => Some(*generation),
                _ => None,
            })
            .unwrap();
        let app = Item::path(
            "/Applications/Calculator.app".into(),
            "Calculator".into(),
            Vec::new(),
        );
        let apps = CommandId::new(crate::features::apps::APPS).unwrap();
        session.deliver_from(
            apps.clone(),
            Delivery::Batch {
                generation,
                items: vec![app],
            },
        );
        session.deliver_from(apps, Delivery::Done { generation });
    }

    /// What the panel moves between as `enter` takes it into a command: whether it animates the
    /// change, and the commands whose results lists, keyed by command, it swaps.
    fn transition(enter: impl FnOnce(&mut Session)) -> (bool, CommandId, CommandId) {
        let mut session = session();
        let mut shown = session.command_id().clone();
        let from = shown.clone();
        enter(&mut session);
        (entered(&mut shown, &session), from, session.command_id().clone())
    }

    #[test]
    fn enter_on_a_lower_command_row_moves_as_keyword_and_space_do() {
        let space = transition(|session| {
            typed(session, "cal");
            session.intent(Intent::Space).unwrap();
        });
        let row = transition(|session| {
            typed(session, "ca");
            assert_eq!(session.items()[0].display_title(), "Calculator");
            session.intent(Intent::Down).unwrap();
            assert_eq!(session.selected_item().unwrap().display_title(), "Calendar");
            session.intent(Intent::Open).unwrap();
        });
        assert_eq!(row, space);
        let (animates, from, to) = space;
        assert!(animates);
        assert_ne!(from, to);
    }

    #[test]
    fn refining_the_query_keeps_the_list_in_place() {
        let mut session = session();
        let mut shown = session.command_id().clone();
        typed(&mut session, "ca");
        typed(&mut session, "cal");
        assert!(!entered(&mut shown, &session));
        session.intent(Intent::Space).unwrap();
        assert!(entered(&mut shown, &session));
        assert!(!entered(&mut shown, &session), "only the change into it moves");
        session.intent(Intent::Backspace).unwrap();
        assert!(entered(&mut shown, &session), "⌫ out of it moves back");
    }
}
