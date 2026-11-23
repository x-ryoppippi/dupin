use std::path::PathBuf;

use rune::Tx;

use super::{Busy, Ui};

impl Ui {
    pub(super) fn copy_files(&mut self, paths: &[PathBuf], tx: &mut Tx) {
        if let Err(error) = dupin_mac::clipboard::copy_files(paths) {
            self.show_notice(error, tx);
        }
    }

    pub(super) fn request_clipboard_access(&self) {
        let busy = Busy::new(self.running.clone());
        std::thread::spawn(move || {
            dupin_mac::clipboard::request_access();
            drop(busy);
        });
    }

    pub(super) fn delete_clipboard_entry(&mut self, id: &str, entry: u64) {
        self.action_screen = None;
        self.clipboard.delete(entry);
        self.session.forget(id);
    }

    pub(super) fn clear_clipboard_history(&mut self, tx: &mut Tx) {
        self.action_screen = None;
        self.clipboard.clear();
        let effects = self.session.refresh();
        self.apply(effects, tx);
        self.timed_notice("Cleared the clipboard history".into(), tx);
    }
}
