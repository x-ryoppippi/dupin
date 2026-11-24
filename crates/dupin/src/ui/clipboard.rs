use std::path::PathBuf;

use dupin_core::clipboard::Content;
use rune::Tx;

use super::{Busy, PendingPaste, Ui};

const PASTE_ACCESS_NOTICE: &str = "Copied. To paste into the front app as well, allow Dupin in System Settings › Privacy & Security › Accessibility.";

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

    /// Paste an extension's text once the panel closes if access is granted, or copy it now.
    pub(super) fn run_paste(&mut self, text: String, tx: &mut Tx) {
        if !rune::tooling::is_headless() && self.may_paste(PASTE_ACCESS_NOTICE) {
            self.paste_after_close = Some(PendingPaste::Text(text));
        } else {
            tx.copy(text);
        }
    }

    /// Copy the entry back now, or paste it once the panel closes if access is granted.
    pub(super) fn restore_clipboard_entry(&mut self, entry: u64, paste: bool, tx: &mut Tx) {
        if paste && !rune::tooling::is_headless() && self.may_paste(PASTE_ACCESS_NOTICE) {
            self.paste_after_close = Some(PendingPaste::ClipboardEntry(entry));
        } else {
            self.deliver_clipboard_entry(entry, false, tx);
        }
    }

    pub(super) fn deliver_clipboard_entry(&mut self, entry: u64, paste: bool, tx: &mut Tx) {
        let Some(record) = self.clipboard.record(entry) else {
            self.show_notice("That copy is no longer in the clipboard history.".into(), tx);
            return;
        };
        if !paste {
            let copied = match &record.content {
                Content::Text(text) => {
                    tx.copy(text.clone());
                    Ok(())
                }
                content => dupin_mac::clipboard::write(content),
            };
            if let Err(error) = copied {
                self.show_notice(error, tx);
            }
            return;
        }
        let me = self.me.clone();
        let busy = Busy::new(self.running.clone());
        let key = dupin_mac::paste::key_code();
        std::thread::spawn(move || {
            let pasted =
                dupin_mac::clipboard::write(&record.content).and_then(|()| dupin_mac::paste::press(key));
            if let Err(error) = pasted {
                me.update(move |ui, tx| ui.show_notice(error, tx));
            }
            drop(busy);
        });
    }

    pub(super) fn delete_clipboard_entry(&mut self, id: &str, entry: u64, tx: &mut Tx) {
        self.action_screen = None;
        let deleted = self.clipboard.delete(entry);
        self.session.forget(id);
        if let Err(error) = deleted {
            crate::log::warn(&format!("couldn't delete the stored clipboard entry: {error}"));
            self.show_notice("Couldn't delete the stored copy of that entry.".into(), tx);
        }
    }

    pub(super) fn clear_clipboard_history(&mut self, tx: &mut Tx) {
        self.action_screen = None;
        let cleared = self.clipboard.clear();
        let effects = self.session.refresh();
        self.apply(effects, tx);
        match cleared {
            Ok(()) => self.timed_notice("Cleared the clipboard history".into(), tx),
            Err(error) => {
                crate::log::warn(&format!("couldn't clear the stored clipboard history: {error}"));
                self.show_notice("Couldn't clear the stored clipboard history.".into(), tx);
            }
        }
    }
}
