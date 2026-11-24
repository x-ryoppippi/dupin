use dupin_core::{Effect, Intent};
use rune::{Key, KeyEvent, Tx};

use super::Ui;

impl Ui {
    /// Keep panel shortcuts available while the definition owns the text selection.
    pub fn unfocused_key(&mut self, ev: &KeyEvent, tx: &mut Tx) -> bool {
        if self.picker.is_some() {
            return self.key(ev, tx);
        }
        if self.action_screen.is_some() {
            return self.key(ev, tx);
        }
        if tx.focused().is_some() {
            return false;
        }
        if ev.key == Key::Backspace && !self.query.text.is_empty() {
            tx.focus("query", false);
            self.query_edit().key(ev, None);
            self.edited(tx);
            return true;
        }
        if let Key::Text(text) = &ev.key
            && !(ev.mods.cmd || ev.mods.ctrl || ev.mods.alt)
        {
            tx.focus("query", false);
            self.query_edit().insert(text);
            self.edited(tx);
            return true;
        }
        self.key(ev, tx)
    }

    /// Keys the field sees first. True consumes the key.
    pub fn key(&mut self, ev: &KeyEvent, tx: &mut Tx) -> bool {
        if self.picker.is_some() {
            return self.picker_key(ev, tx);
        }
        if ev.key == Key::Enter && self.caller_hold.holds(tx.clock()) {
            return true;
        }
        if self.script_entry.is_some() {
            return self.script_entry_key(ev, tx);
        }
        if self.script_card_key(ev, tx) {
            return true;
        }
        if ev.mods.cmd && ev.mods.shift && matches!(ev.key, Key::Command('s') | Key::Command('S')) {
            self.capture_snapshot(tx);
            return true;
        }
        if ev.mods.cmd && ev.mods.shift && matches!(ev.key, Key::Command('o') | Key::Command('O')) {
            self.show_snapshots(tx);
            return true;
        }
        if !self.session.command().private_results {
            crate::trace::key(ev);
        }
        if let Some(preview) = self.quick_look.borrow().as_ref() {
            preview.trace_picker_input();
        }
        match &ev.key {
            // An accessory app has no menu bar, so ⌘Q in the panel is the way to quit.
            Key::Command('q') => {
                self.quit(tx);
                return true;
            }
            Key::Command(',') => {
                self.prepare_settings();
                self.dismiss(tx);
                return true;
            }
            _ => {}
        }
        if self.action_screen_key(ev, tx) {
            return true;
        }
        if self.query.is_composing() {
            return false;
        }
        // Held Enter must not confirm an operation whose prompt it just opened.
        if ev.key == Key::Enter && tx.key_repeat() {
            return true;
        }
        if self.session.destination().is_none() && is_quick_look_shortcut(&ev.key) {
            if !self
                .session
                .selected_item()
                .is_some_and(|item| item.item_action(dupin_mac::quick_look::ACTION_ID).is_some())
            {
                return false;
            }
            let key = self.session.result_key(self.session.selected());
            let effects = self
                .session
                .run_key_action(&key, dupin_mac::quick_look::ACTION_ID);
            self.apply(effects, tx);
            return true;
        }
        let m = ev.mods;
        let plain = !(m.cmd || m.ctrl || m.alt);
        if plain
            && !m.shift
            && matches!(ev.key, Key::Tab | Key::Right)
            && self.query.anchor == self.query.focus
            && self.query.focus == self.query.text.len()
            && self
                .session
                .selected_item()
                .is_some_and(|item| item.action(crate::scripts::provider::ARGUMENTS).is_some())
        {
            let key = self.session.result_key(self.session.selected());
            let effects = self
                .session
                .run_key_action(&key, crate::scripts::provider::ARGUMENTS);
            self.apply(effects, tx);
            return true;
        }
        let intent = match &ev.key {
            Key::Tab
                if plain
                    && !m.shift
                    && self.query.anchor == self.query.focus
                    && self.query.focus == self.query.text.len() =>
            {
                Intent::Complete
            }
            Key::Text(t) if t == " " && plain && !m.shift && self.query.anchor == self.query.focus => {
                Intent::Space
            }
            Key::Backspace if plain => Intent::Backspace,
            Key::Up => Intent::Up,
            Key::Down => Intent::Down,
            Key::Right
                if plain
                    && !m.shift
                    && self.query.anchor == self.query.focus
                    && self.query.focus == self.query.text.len() =>
            {
                Intent::Actions
            }
            Key::Command('k') => Intent::Actions,
            Key::Text(t) if m.ctrl && t.eq_ignore_ascii_case("k") => Intent::Actions,
            Key::Enter if m.cmd => Intent::OpenAlternate,
            Key::Enter => Intent::Open,
            Key::Escape => Intent::Dismiss,
            _ => return false,
        };
        let Some(effects) = self.session.intent(intent) else {
            return false;
        };
        self.apply(effects, tx);
        true
    }

    /// A click on a result row.
    pub fn open(&mut self, id: &str, tx: &mut Tx) {
        if self.picker.is_some() || self.caller_hold.holds(tx.clock()) {
            return;
        }
        if self.session.select_key(id) {
            if self.session.destination().is_some() {
                tx.focus("query", false);
                self.apply(vec![Effect::RevealSelection], tx);
            } else if let Some(effects) = self.session.intent(Intent::Open) {
                self.apply(effects, tx);
            }
        }
    }

    pub fn open_link(&mut self, id: &str, url: &str, tx: &mut Tx) {
        if self.picker.is_some() || self.session.destination().is_some() {
            return;
        }
        let effects = self.session.open_link(id, url);
        self.apply(effects, tx);
    }
}

fn is_quick_look_shortcut(key: &Key) -> bool {
    matches!(key, Key::Command('y'))
}

#[cfg(test)]
#[test]
fn quick_look_shortcut_is_command_y() {
    assert!(is_quick_look_shortcut(&Key::Command('y')));
    assert!(!is_quick_look_shortcut(&Key::Command('k')));
    assert!(!is_quick_look_shortcut(&Key::Text("y".into())));
}
