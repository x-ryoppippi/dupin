use std::path::PathBuf;

use dupin_extension_bridge::{ClipboardAccess, OsAccess, OsPermission, Request, Trigger};
use rune::Tx;

use super::snippets::RowText;
use super::{Busy, PendingPaste, Ui};
use crate::extensions::Extension;

const PASTE_ACCESS_NOTICE: &str = "Copied. To paste into the front app as well, allow Dupin in System Settings › Privacy & Security › Accessibility.";

impl Ui {
    pub(super) fn copy_files(&mut self, paths: &[PathBuf], tx: &mut Tx) {
        if let Err(error) = dupin_mac::clipboard::copy_files(paths) {
            self.show_notice(error, tx);
        }
    }

    /// Follows `[clipboard]`: the settings the clipboard service reads, the history's
    /// clipboard watch, and what it stored, deleted when it is turned off. Returns whether it
    /// was turned on or off.
    pub(crate) fn apply_clipboard(&mut self, config: &dupin_core::config::Clipboard) -> bool {
        let toggled = self.clipboard.apply(config, &self.extensions.bridge);
        if toggled {
            self.install_declarations();
        }
        toggled
    }

    /// Presses ⌘V in the app in front, once the panel has closed, while `permit` still holds.
    pub(super) fn paste_clipboard(&self, permit: &dupin_extension_bridge::Permit) {
        if !self.extensions.bridge.validate(permit) {
            return;
        }
        let me = self.me.clone();
        let busy = Busy::new(self.running.clone());
        let key = dupin_mac::paste::key_code();
        std::thread::spawn(move || {
            if let Err(error) = dupin_mac::paste::press(key) {
                me.update(move |ui, tx| ui.show_notice(error, tx));
            }
            drop(busy);
        });
    }

    /// Closes the panel after an extension's action put something on the clipboard and, when
    /// it asked to paste, pastes it into the app in front, as a row's own paste does: while the
    /// extension holds `clipboard:paste` and macOS lets Dupin post ⌘V.
    pub(super) fn close_after_copy(&mut self, extension: &Extension, paste: bool, tx: &mut Tx) {
        if paste {
            let request = Request::Clipboard(ClipboardAccess::Paste);
            match self
                .extensions
                .bridge
                .authorize(&extension.package, &request, Trigger::Activation)
            {
                Ok(permit) if !rune::tooling::is_headless() && self.may_paste(PASTE_ACCESS_NOTICE) => {
                    self.paste_after_close = Some(PendingPaste::Clipboard { permit });
                }
                Ok(_) => {}
                Err(refused) => self.notice_after_close = Some(format!("Copied instead. {refused}")),
            }
        }
        self.hide(tx);
    }

    /// Paste an extension row's text once the panel closes, while the extension holds
    /// `clipboard:paste` and macOS lets Dupin post ⌘V, or copy it now. A refusal for want of
    /// consent waits in the panel and Settings, as a request from the extension's code would.
    /// A built-in's port pastes as the built-in did: Dupin asks macOS for its own access on
    /// first use and explains a refusal with the built-in's notice.
    pub(super) fn run_paste(&mut self, extension: &Extension, text: String, template: bool, tx: &mut Tx) {
        let text = RowText::new(text, template);
        if let Some(notice) = crate::features::paste_notice(&extension.package) {
            if rune::tooling::is_headless() || !self.may_paste(notice) {
                return self.hand_over(text, false, tx);
            }
            self.extensions
                .bridge
                .set_os_access(OsPermission::Accessibility, OsAccess::Allowed);
        }
        let paste = Request::Clipboard(ClipboardAccess::Paste);
        match self
            .extensions
            .bridge
            .authorize(&extension.package, &paste, Trigger::Activation)
        {
            Ok(permit) if !rune::tooling::is_headless() && self.may_paste(PASTE_ACCESS_NOTICE) => {
                self.paste_after_close = Some(PendingPaste::Extension { text, permit });
            }
            Ok(_) => self.hand_over(text, false, tx),
            Err(refused) => {
                self.hand_over(text, false, tx);
                self.notice_after_close = Some(format!("Copied instead. {refused}"));
            }
        }
    }

    pub(super) fn request_clipboard_access(&self) {
        let busy = Busy::new(self.running.clone());
        std::thread::spawn(move || {
            dupin_mac::clipboard::request_access();
            drop(busy);
        });
    }
}
