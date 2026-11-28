use dupin_core::{Effect, Intent};
use rune::{Key, KeyEvent, PhysicalKeyCode, Shortcut, Tx};

use super::Ui;

impl Ui {
    /// Keep panel shortcuts available while the definition owns the text selection.
    pub fn unfocused_key(&mut self, ev: &KeyEvent, tx: &mut Tx) -> bool {
        if self.picker.is_some() {
            return self.key(ev, tx);
        }
        if self.form_draft().is_some() {
            // A focused field had the key first and passed it on.
            return tx.focused().is_none() && self.form_key(ev, tx);
        }
        if self.action_screen.is_some() {
            return self.key(ev, tx);
        }
        if tx.focused().is_some() {
            return false;
        }
        if self.navigation_key(ev, tx).is_some() {
            return self.key(ev, tx);
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
        if !self.session.command().private_results {
            crate::trace::key(ev);
        }
        let mapped;
        let ev = match self.navigation_key(ev, tx) {
            Some(key) => {
                mapped = KeyEvent { key, ..ev.clone() };
                &mapped
            }
            None => ev,
        };
        let flipped;
        let ev = match list_key(&ev.key, self.grows_up()) {
            Some(key) => {
                flipped = KeyEvent { key, ..ev.clone() };
                &flipped
            }
            None => ev,
        };
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
                .is_some_and(|item| item.item_action(dupin_core::kind::QUICK_LOOK).is_some())
            {
                return false;
            }
            let key = self.session.result_key(self.session.selected());
            let effects = self.session.run_key_action(&key, dupin_core::kind::QUICK_LOOK);
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
        let direction = match intent {
            Intent::Up => Some("up"),
            Intent::Down => Some("down"),
            _ => None,
        };
        let previous = self.session.selected();
        let Some(effects) = self.session.intent(intent) else {
            return false;
        };
        self.apply(effects, tx);
        if let Some(direction) = direction {
            crate::trace::navigation(direction, previous, self.session.selected());
        }
        true
    }

    /// Optional alternatives to the arrow keys, shared by the result, action and picker lists.
    fn navigation_key(&self, ev: &KeyEvent, tx: &Tx) -> Option<Key> {
        if self.script_entry.is_some() || self.script_card.is_some() || self.form_draft().is_some() {
            return None;
        }
        let config = self.configuration.values.borrow();
        configured_navigation_key(
            &config.navigation_up,
            &config.navigation_down,
            ev,
            tx.physical_key(),
        )
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

/// Recorded settings name physical keys, so matching must use the same identity on every layout.
fn configured_navigation_key(
    up: &str,
    down: &str,
    ev: &KeyEvent,
    physical_key: Option<PhysicalKeyCode>,
) -> Option<Key> {
    if matches!(ev.key, Key::Up | Key::Down) {
        return None;
    }
    if navigation_shortcut_matches(up, ev, physical_key) {
        Some(Key::Up)
    } else if navigation_shortcut_matches(down, ev, physical_key) {
        Some(Key::Down)
    } else {
        None
    }
}

fn navigation_shortcut_matches(value: &str, ev: &KeyEvent, physical_key: Option<PhysicalKeyCode>) -> bool {
    value.parse::<Shortcut>().is_ok_and(|shortcut| {
        physical_key == Some(shortcut.physical_key()) && shortcut.modifiers() == ev.mods
    })
}

/// ↑ and ↓ as the lists see them: a list that runs upwards from the field shows its next row
/// above, so ↑ moves to it. `None` leaves the key as it is.
fn list_key(key: &Key, grows_up: bool) -> Option<Key> {
    match key {
        Key::Up if grows_up => Some(Key::Down),
        Key::Down if grows_up => Some(Key::Up),
        _ => None,
    }
}

fn is_quick_look_shortcut(key: &Key) -> bool {
    matches!(key, Key::Command('y'))
}

#[cfg(test)]
#[test]
fn arrows_follow_a_list_that_runs_upwards() {
    assert_eq!(list_key(&Key::Up, true), Some(Key::Down));
    assert_eq!(list_key(&Key::Down, true), Some(Key::Up));
    assert_eq!(list_key(&Key::Up, false), None);
    assert_eq!(list_key(&Key::Down, false), None);
    assert_eq!(list_key(&Key::Enter, true), None);
}

#[cfg(test)]
#[test]
fn quick_look_shortcut_is_command_y() {
    assert!(is_quick_look_shortcut(&Key::Command('y')));
    assert!(!is_quick_look_shortcut(&Key::Command('k')));
    assert!(!is_quick_look_shortcut(&Key::Text("y".into())));
}

#[cfg(test)]
#[test]
fn navigation_shortcuts_match_the_recorded_physical_key() {
    use rune::{KeyEvent, Mods};

    let event = KeyEvent {
        key: Key::Text("z".into()),
        mods: Mods {
            ctrl: true,
            cmd: !cfg!(target_os = "macos"),
            ..Mods::default()
        },
    };
    assert_eq!(
        configured_navigation_key("", "ctrl+KeyY", &event, Some(PhysicalKeyCode::KeyY)),
        Some(Key::Down),
        "a layout-dependent character still matches the physical key recorded by the shortcut",
    );
    assert_eq!(
        configured_navigation_key("", "ctrl+KeyY", &event, Some(PhysicalKeyCode::KeyZ)),
        None,
        "a different physical key does not match just because of its typed character",
    );
}

#[cfg(test)]
#[test]
fn configured_navigation_does_not_remap_native_arrows() {
    let event = KeyEvent {
        key: Key::Down,
        mods: rune::Mods::default(),
    };
    assert_eq!(
        configured_navigation_key("up", "", &event, Some(PhysicalKeyCode::ArrowDown)),
        None,
    );
}
