use rune::Tx;

use super::{Busy, Ui};

const PASTE_ACCESS_NOTICE: &str = "Copied the snippet. To paste it into the front app as well, allow Dupin in System Settings › Privacy & Security › Accessibility, or turn off Paste snippets in Settings › Search.";

/// A snippet's text as Enter found it, with the time its placeholders show.
pub(super) struct SnippetRun {
    text: String,
    now: dupin_core::snippets::LocalTime,
}

impl Ui {
    /// Copy the snippet now, or, to paste it, once the panel closes. Pasting needs
    /// Accessibility access: the first attempt without it asks macOS, and later ones explain.
    pub(super) fn run_snippet(&mut self, text: String, paste: bool, tx: &mut Tx) {
        let snippet = SnippetRun {
            text,
            now: dupin_mac::clock::now(),
        };
        if paste && !rune::tooling::is_headless() && self.may_paste() {
            self.paste_after_close = Some(snippet);
        } else {
            self.deliver_snippet(snippet, false, tx);
        }
    }

    fn may_paste(&mut self) -> bool {
        if dupin_mac::paste::allowed() {
            return true;
        }
        if std::mem::replace(&mut self.paste_access_asked, true) {
            self.notice_after_close = Some(PASTE_ACCESS_NOTICE.into());
        } else {
            dupin_mac::paste::request();
        }
        false
    }

    pub(super) fn deliver_snippet(&self, snippet: SnippetRun, paste: bool, tx: &mut Tx) {
        use dupin_core::snippets::{expand, needs_clipboard};

        if needs_clipboard(&snippet.text) {
            let busy = Busy::new(self.running.clone());
            tx.read_clipboard(move |ui: &mut Self, content, tx| {
                let text = expand(&snippet.text, snippet.now, content.text());
                ui.finish_snippet(text, paste, tx);
                drop(busy);
            });
        } else {
            let text = expand(&snippet.text, snippet.now, "");
            self.finish_snippet(text, paste, tx);
        }
    }

    fn finish_snippet(&self, text: String, paste: bool, tx: &mut Tx) {
        if !paste {
            tx.copy(text);
            return;
        }
        let me = self.me.clone();
        let busy = Busy::new(self.running.clone());
        let key = dupin_mac::paste::key_code();
        std::thread::spawn(move || {
            if let Err(error) = dupin_mac::paste::paste(key, &text) {
                me.update(move |ui, tx| ui.show_notice(error, tx));
            }
            drop(busy);
        });
    }
}
