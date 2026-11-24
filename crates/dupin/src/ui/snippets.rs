use rune::Tx;

use super::{Busy, PendingPaste, Ui};

const PASTE_ACCESS_NOTICE: &str = "Copied the snippet. To paste it into the front app as well, allow Dupin in System Settings › Privacy & Security › Accessibility, or turn off Paste snippets in Settings › Search.";

/// A snippet's text as Enter found it, with the time its placeholders show.
pub(super) struct SnippetRun {
    text: String,
    now: dupin_core::snippets::LocalTime,
}

impl SnippetRun {
    fn now(text: String) -> Self {
        Self {
            text,
            now: dupin_mac::clock::now(),
        }
    }
}

/// Text an extension row copies or pastes: as it is, or a template filled in as a snippet is.
pub(super) enum RowText {
    Plain(String),
    Template(SnippetRun),
}

impl RowText {
    pub(super) fn new(text: String, template: bool) -> Self {
        if template {
            Self::Template(SnippetRun::now(text))
        } else {
            Self::Plain(text)
        }
    }
}

impl Ui {
    /// Copy the snippet now, or paste it once the panel closes if access is granted.
    pub(super) fn run_snippet(&mut self, text: String, paste: bool, tx: &mut Tx) {
        let snippet = SnippetRun::now(text);
        if paste && !rune::tooling::is_headless() && self.may_paste(PASTE_ACCESS_NOTICE) {
            self.paste_after_close = Some(PendingPaste::Snippet(snippet));
        } else {
            self.deliver_snippet(snippet, false, tx);
        }
    }

    /// Whether Dupin may paste for the user; when it may not, `notice` explains why once the
    /// panel has closed, unless macOS is asking right now.
    pub(super) fn may_paste(&mut self, notice: &str) -> bool {
        if dupin_mac::paste::allowed() {
            return true;
        }
        if !dupin_mac::paste::request_once() {
            self.notice_after_close = Some(notice.into());
        }
        false
    }

    /// Copies the text, or pastes it into the frontmost app, which must already be in front.
    pub(super) fn hand_over(&self, text: RowText, paste: bool, tx: &mut Tx) {
        match text {
            RowText::Template(snippet) => self.deliver_snippet(snippet, paste, tx),
            RowText::Plain(text) => self.finish_snippet(text, paste, tx),
        }
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
        if paste {
            self.paste_text(text);
        } else {
            tx.copy(text);
        }
    }

    /// Pastes `text` into the frontmost app, which must already be in front.
    pub(super) fn paste_text(&self, text: String) {
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
