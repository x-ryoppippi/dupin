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

pub fn settings(event: &str) {
    if on() {
        eprintln!(
            "dupin-trace: settings {event} activation={:?} wall={:.3}",
            dupin_mac::window::application_activation(),
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

/// A key the query field saw, before Dupin's handler decides on it.
pub fn key(ev: &rune::KeyEvent) {
    if on() {
        eprintln!("dupin-trace: key {:?} wall={:.3}", ev.key, wall_ms());
    }
}

/// The query field's text after an edit.
pub fn edited(text: &str) {
    if on() {
        eprintln!("dupin-trace: edited {text:?} wall={:.3}", wall_ms());
    }
}
