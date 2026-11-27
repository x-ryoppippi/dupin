use std::path::{Path, PathBuf};

use dupin_core::item::Transfer;
use rune::Tx;

use super::{
    Busy, Ui,
    notices::{FileAction, FileOperation},
};

impl Ui {
    pub(super) fn select_rename_stem(&mut self, path: &Path, tx: &mut Tx) {
        let extension = dupin_mac::file_ops::has_extension(path);
        let end = dupin_core::file_name::stem_len(&self.query.text, extension);
        tx.focus("query", false);
        self.query.set_selection(0, end);
    }

    pub(super) fn rename_file(&self, row: String, path: PathBuf, name: String) {
        let kinds = self.kinds.clone();
        self.on_file_worker(
            FileAction::Rename,
            path,
            move |path| {
                dupin_mac::file_ops::rename(path, &name)
                    .map(|renamed| dupin_mac::kinds::file(&kinds, renamed, name, Vec::new()))
            },
            move |ui, _, item| {
                let notice = format!("Renamed to “{}”", item.display_title());
                ui.session.replace(&row, item);
                if ui
                    .action_screen
                    .as_ref()
                    .is_some_and(|screen| !screen.is_current(&ui.session))
                {
                    ui.action_screen = None;
                }
                ui.fetch_icons();
                notice
            },
        );
    }

    pub(super) fn transfer_file(&self, transfer: Transfer, path: PathBuf, folder: PathBuf) {
        let destination = file_name(&folder);
        let files = self.file_transfers.clone();
        self.on_file_worker(
            FileAction::Transfer(transfer),
            path,
            move |path| match transfer {
                Transfer::Move => files.move_to(path, &folder),
                Transfer::Copy => files.copy_to(path, &folder),
            },
            move |ui, path, output| {
                if transfer == Transfer::Move {
                    ui.session.forget_path(path);
                    if ui
                        .action_screen
                        .as_ref()
                        .is_some_and(|screen| !screen.is_current(&ui.session))
                    {
                        ui.action_screen = None;
                    }
                }
                let done = match transfer {
                    Transfer::Move => "Moved",
                    Transfer::Copy => "Copied",
                };
                format!("{done} “{}” to “{destination}”", file_name(&output))
            },
        );
    }

    pub(super) fn duplicate_file(&self, path: PathBuf) {
        let files = self.file_transfers.clone();
        self.on_file_worker(
            FileAction::Duplicate,
            path,
            move |path| files.duplicate(path),
            |_, _, copy| format!("Duplicated as “{}”", file_name(&copy)),
        );
    }

    pub(super) fn compress_file(&self, path: PathBuf) {
        self.on_file_worker(
            FileAction::Compress,
            path,
            dupin_mac::file_ops::compress,
            |_, _, archive| format!("Compressed to “{}”", file_name(&archive)),
        );
    }

    pub(super) fn trash_file(&mut self, id: String, path: PathBuf) {
        self.action_screen = None;
        self.on_file_worker(
            FileAction::Trash,
            path,
            |path| dupin_mac::workspace::run(&dupin_mac::workspace::Action::Trash(path.to_owned())),
            move |ui, path, ()| {
                ui.session.forget(&id);
                format!("Moved “{}” to the Trash", file_name(path))
            },
        );
    }

    /// Run a file operation off the main thread and report its result back on it. A relaunch
    /// waits until it has finished.
    fn on_file_worker<T: Send + 'static>(
        &self,
        action: FileAction,
        path: PathBuf,
        work: impl FnOnce(&Path) -> Result<T, String> + Send + 'static,
        done: impl FnOnce(&mut Ui, &Path, T) -> String + Send + 'static,
    ) {
        let me = self.me.clone();
        let busy = Busy::new(self.running.clone());
        let operation = FileOperation { action, path };
        std::thread::spawn(move || {
            let result = work(&operation.path);
            me.update(move |ui, tx| match result {
                Ok(output) => {
                    let message = done(ui, &operation.path, output);
                    ui.show_timed_notice(message, Some(&operation), tx);
                }
                Err(error) => {
                    let name = file_name(&operation.path);
                    let what = match operation.action {
                        FileAction::Rename => format!("rename “{name}”"),
                        FileAction::Transfer(Transfer::Move) => format!("move “{name}”"),
                        FileAction::Transfer(Transfer::Copy) => format!("copy “{name}”"),
                        FileAction::Duplicate => format!("duplicate “{name}”"),
                        FileAction::Compress => format!("compress “{name}”"),
                        FileAction::Trash => format!("move “{name}” to the Trash"),
                    };
                    ui.show_persistent_notice(format!("Couldn't {what}: {error}"), Some(operation), tx);
                }
            });
            drop(busy);
        });
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}
