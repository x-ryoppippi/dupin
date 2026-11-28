//! `DUPIN_TRACE=1`: print when the launch hotkey is registered, how long each summon takes,
//! from the hotkey handler to the first frame drawn after it, every drawn frame's offset, and
//! each key and edit the query field sees, to stderr. Off, it costs one branch.

use std::cell::Cell;
use std::sync::OnceLock;
use std::time::Instant;

use rune::{FrameEncoded, FrameOutcome};

fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("DUPIN_TRACE").is_some_and(|v| v != "0"))
}

pub fn action_screen(stage: &str) {
    if on() {
        eprintln!("dupin-trace: action-screen {stage}");
    }
}

pub fn settings(event: &str) {
    if on() {
        eprintln!("dupin-trace: settings {event} wall={:.3}", wall_ms());
    }
}

pub fn results_done(query: &str, items: usize) {
    if on() {
        eprintln!("dupin-trace: results done query={query:?} items={items}");
    }
}

/// A navigation key changed the selected result; the trace contains no result text.
pub fn navigation(direction: &str, previous: usize, selected: usize) {
    if on() && previous != selected {
        eprintln!("dupin-trace: navigation {direction} moved selection {previous}->{selected}");
    }
}

pub fn snapshot(event: &str) {
    if on() {
        eprintln!("dupin-trace: snapshot {event}");
    }
}

pub fn control(kind: &str, ok: bool) {
    if on() {
        eprintln!("dupin-trace: control {kind} ok={ok}");
    }
}

pub fn panel(event: &str, visible: bool, shown: bool) {
    if on() {
        eprintln!(
            "dupin-trace: panel {event} visible={visible} shown={shown} wall={:.3}",
            wall_ms()
        );
    }
}

thread_local! {
    static SUMMONED: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// The hotkey is live: a key press from now on summons the panel.
pub fn registered(accelerator: &str) {
    if on() {
        eprintln!(
            "dupin-trace: hotkey {accelerator} registered wall={:.3}",
            wall_ms()
        );
    }
}

/// The hotkey asked for the panel.
pub fn summoned() {
    if on() {
        SUMMONED.set(Some(Instant::now()));
        eprintln!("dupin-trace: summon wall={:.3}", wall_ms());
    }
}

/// `App::on_frame_encoded`.
pub fn frame(f: &FrameEncoded) {
    if !on() {
        return;
    }
    let Some(at) = SUMMONED.get() else { return };
    let ms = f.at().saturating_duration_since(at).as_secs_f64() * 1000.0;
    eprintln!(
        "dupin-trace: frame {:?} at +{ms:.1} ms wall={:.3}",
        f.outcome(),
        wall_ms()
    );
    if f.outcome() == FrameOutcome::Drawn && ms > 0.0 {
        SUMMONED.set(None);
    }
}

/// Milliseconds since the Unix epoch, to line the trace up with other processes' clocks.
fn wall_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

/// A global hotkey reached Dupin.
pub fn hotkey(shortcut: &str) {
    if on() {
        eprintln!("dupin-trace: hotkey fired {shortcut} wall={:.3}", wall_ms());
    }
}

/// A key the panel saw, before Dupin's handler decides on it.
pub fn key(ev: &rune::KeyEvent) {
    if on() {
        eprintln!("dupin-trace: key {} wall={:.3}", key_label(ev), wall_ms());
    }
}

/// The key's kind and modifiers. Typed text loses its characters: the key can belong to a
/// script argument or a picker filter, and the trace ends up in uploaded e2e artifacts.
fn key_label(ev: &rune::KeyEvent) -> String {
    let key = match &ev.key {
        rune::Key::Text(_) => "Text".to_owned(),
        key => format!("{key:?}"),
    };
    let m = ev.mods;
    let mods: Vec<&str> = [
        (m.cmd, "cmd"),
        (m.ctrl, "ctrl"),
        (m.alt, "alt"),
        (m.shift, "shift"),
    ]
    .into_iter()
    .filter_map(|(held, name)| held.then_some(name))
    .collect();
    if mods.is_empty() {
        key
    } else {
        format!("{key} mods={}", mods.join("+"))
    }
}

/// The query field's text after an edit.
pub fn edited(text: &str) {
    if on() {
        eprintln!("dupin-trace: edited {text:?} wall={:.3}", wall_ms());
    }
}

#[cfg(test)]
mod tests {
    use rune::{Key, KeyEvent, Mods};

    use super::key_label;

    #[test]
    fn typed_text_is_traced_without_its_characters() {
        let ev = KeyEvent {
            key: Key::Text("hunter2".into()),
            mods: Mods {
                shift: true,
                ..Mods::default()
            },
        };
        assert_eq!(key_label(&ev), "Text mods=shift");
    }

    #[test]
    fn chords_keep_their_key_and_modifiers() {
        let chord = KeyEvent {
            key: Key::Command('s'),
            mods: Mods {
                cmd: true,
                shift: true,
                ..Mods::default()
            },
        };
        assert_eq!(key_label(&chord), "Command('s') mods=cmd+shift");
        let bare = KeyEvent {
            key: Key::Left,
            mods: Mods::default(),
        };
        assert_eq!(key_label(&bare), "Left");
    }
}
