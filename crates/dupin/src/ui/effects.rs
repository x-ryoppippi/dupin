use std::path::Path;

use dupin_core::{Action, Effect};
use rune::Tx;
use rune::raw_window_handle::RawWindowHandle;

use super::{Busy, Ui, query::mark_session_query};

fn same_bundle_path(path: &Path, running_bundle: &Path) -> bool {
    let resolved = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_owned());
    resolved(path) == resolved(running_bundle)
}

impl Ui {
    pub(super) fn apply(&mut self, effects: Vec<Effect>, tx: &mut Tx) {
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
                    Action::Open(path) => {
                        let running_bundle = dupin_mac::bundle::path();
                        if same_bundle_path(&path, &running_bundle) {
                            // Launching Dupin itself cannot show a normal window; take the user to Preferences.
                            self.prepare_settings();
                        } else {
                            self.run_workspace_action(dupin_mac::workspace::Action::Open(path));
                        }
                    }
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

#[cfg(test)]
mod tests {
    use super::same_bundle_path;
    use std::os::unix::fs::symlink;

    #[test]
    fn recognizes_the_running_bundle_through_an_application_link() {
        let directory = tempfile::tempdir().unwrap();
        let bundle = directory.path().join("Dupin.app");
        let link = directory.path().join("Applications/Dupin.app");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        symlink(&bundle, &link).unwrap();

        assert!(same_bundle_path(&bundle, &bundle));
        assert!(same_bundle_path(&link, &bundle));
        assert!(!same_bundle_path(&directory.path().join("Other.app"), &bundle));
    }
}
