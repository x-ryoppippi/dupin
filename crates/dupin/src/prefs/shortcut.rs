//! A shortcut shown as one compact keycap at a row's trailing edge, which records a new chord
//! when pressed. Every row in Settings that records a shortcut uses it.
//!
//! The capture follows rune-kit's `ShortcutRecorder` (MIT), whose machine is private to its
//! wide, text-field-shaped control.

use std::rc::Rc;

use rune::{El, Mods, PhysicalInput, PhysicalKeyCode, Shortcut, ShortcutError, col, icon, row, text};
use rune_kit::{Kit, shortcut_error_message};
use rune_theme_opencode::icons::CLOSE;

use super::Prefs;

/// The widest a refusal under the keycap wraps at.
const ERROR_MAX_W: f32 = 220.0;

/// A recorded shortcut and its capture: idle, capturing a chord, or swallowing the repeats and
/// release of the key that ended one.
#[derive(Clone, Debug, Default)]
pub(super) struct ShortcutState {
    shortcut: Option<Shortcut>,
    phase: Phase,
    /// The modifiers held now, which a capture shows as it waits for a key.
    held: Mods,
    unregistered: bool,
}

#[derive(Clone, Debug, Default)]
enum Phase {
    #[default]
    Idle,
    /// `refusal` is the last chord refused, until a new chord starts; `finishing` is the key
    /// that ended or refused it.
    Capturing {
        refusal: Option<Refusal>,
        finishing: Option<PhysicalKeyCode>,
    },
    Swallowing {
        key: PhysicalKeyCode,
    },
}

/// Why a chord was refused, and the chord (None for a refused clear or a key that can't be one).
#[derive(Clone, Debug)]
struct Refusal {
    error: ShortcutError,
    chord: Option<Shortcut>,
}

enum Step {
    Pass,
    Consumed,
    Propose(Option<Shortcut>),
}

impl ShortcutState {
    pub(super) fn new(shortcut: Option<Shortcut>) -> Self {
        Self {
            shortcut,
            ..Self::default()
        }
    }

    pub(super) fn shortcut(&self) -> Option<&Shortcut> {
        self.shortcut.as_ref()
    }

    pub(super) fn is_recording(&self) -> bool {
        matches!(self.phase, Phase::Capturing { .. })
    }

    /// The shortcut isn't registered: recording the same chord proposes it again.
    pub(super) fn mark_unregistered(&mut self) {
        self.unregistered = true;
    }

    fn refusal(&self) -> Option<&Refusal> {
        match &self.phase {
            Phase::Capturing { refusal, .. } => refusal.as_ref(),
            _ => None,
        }
    }

    pub(super) fn error(&self) -> Option<&ShortcutError> {
        self.refusal().map(|refusal| &refusal.error)
    }

    /// The chord the last change refused, until another chord starts.
    pub(super) fn refused(&self) -> Option<&Shortcut> {
        self.refusal().and_then(|refusal| refusal.chord.as_ref())
    }

    /// What the keycap shows: the refused chord, the modifiers held while capturing, or the
    /// shortcut, each in Apple's order, ⌃⌥⇧⌘ then the key.
    pub(super) fn keycaps(&self) -> Vec<String> {
        if let Some(chord) = self.refused() {
            chord.keycaps()
        } else if self.is_recording() {
            rune::modifier_keycaps(self.held)
        } else {
            self.shortcut.as_ref().map(Shortcut::keycaps).unwrap_or_default()
        }
    }

    pub(super) fn begin(&mut self) {
        self.phase = Phase::Capturing {
            refusal: None,
            finishing: None,
        };
    }

    fn finishing(&self) -> Option<PhysicalKeyCode> {
        match self.phase {
            Phase::Capturing { finishing, .. } => finishing,
            Phase::Swallowing { key } => Some(key),
            Phase::Idle => None,
        }
    }

    /// Ends the capture, still swallowing the repeats and release of the key that ended it.
    pub(super) fn cancel(&mut self) {
        self.phase = match self.finishing() {
            Some(key) => Phase::Swallowing { key },
            None => Phase::Idle,
        };
        self.held = Mods::default();
    }

    fn refuse(&mut self, error: ShortcutError, chord: Option<Shortcut>) {
        if let Phase::Capturing { refusal, .. } = &mut self.phase {
            *refusal = Some(Refusal { error, chord });
        }
    }

    /// Pressing another modifier starts a new chord, so the refused one and its error go.
    fn hold(&mut self, mods: Mods) {
        let held = self.held;
        let pressed = (mods.ctrl && !held.ctrl)
            || (mods.alt && !held.alt)
            || (mods.shift && !held.shift)
            || (mods.cmd && !held.cmd)
            || (mods.logo && !held.logo);
        if pressed && let Phase::Capturing { refusal, .. } = &mut self.phase {
            *refusal = None;
        }
        self.held = mods;
    }

    fn input(&mut self, event: &PhysicalInput, disabled: bool, require_modifier: bool) -> Step {
        if disabled && !matches!(event, PhysicalInput::FocusLost) {
            return Step::Pass;
        }
        match *event {
            PhysicalInput::FocusLost => {
                let active = !matches!(self.phase, Phase::Idle);
                self.held = Mods::default();
                self.phase = Phase::Idle;
                if active { Step::Consumed } else { Step::Pass }
            }
            PhysicalInput::Modifiers(mods) => {
                self.hold(mods);
                if self.is_recording() {
                    Step::Consumed
                } else {
                    Step::Pass
                }
            }
            PhysicalInput::Key {
                key,
                mods,
                pressed,
                repeat,
                ..
            } => {
                self.hold(mods);
                if self.finishing() == Some(key) {
                    if !pressed {
                        self.phase = match self.phase {
                            Phase::Capturing { ref refusal, .. } => Phase::Capturing {
                                refusal: refusal.clone(),
                                finishing: None,
                            },
                            _ => Phase::Idle,
                        };
                    }
                    return Step::Consumed;
                }
                if !self.is_recording() {
                    return Step::Pass;
                }
                if !pressed || repeat || is_modifier(key) {
                    return Step::Consumed;
                }
                if let Phase::Capturing { finishing, .. } = &mut self.phase {
                    *finishing = Some(key);
                }
                match key {
                    PhysicalKeyCode::Escape => {
                        self.cancel();
                        Step::Consumed
                    }
                    PhysicalKeyCode::Backspace => Step::Propose(None),
                    _ => match Shortcut::from_physical_key(key, mods) {
                        Ok(chord) if !require_modifier => Step::Propose(Some(chord)),
                        Ok(chord) => match chord.require_modifier() {
                            Ok(()) => Step::Propose(Some(chord)),
                            Err(error) => {
                                self.refuse(error, Some(chord));
                                Step::Consumed
                            }
                        },
                        Err(error) => {
                            self.refuse(error, None);
                            Step::Consumed
                        }
                    },
                }
            }
        }
    }

    /// Applies `result`, the registration of `next`: the shortcut on success, or a capture that
    /// shows the refused chord.
    fn finish(&mut self, next: Option<Shortcut>, result: Result<(), ShortcutError>) {
        match result {
            Ok(()) => {
                self.shortcut = next;
                self.unregistered = false;
                self.cancel();
            }
            Err(error) => self.refuse(error, next),
        }
    }
}

fn is_modifier(key: PhysicalKeyCode) -> bool {
    use PhysicalKeyCode::*;
    matches!(
        key,
        ShiftLeft
            | ShiftRight
            | ControlLeft
            | ControlRight
            | AltLeft
            | AltRight
            | SuperLeft
            | SuperRight
            | Fn
            | FnLock
    )
}

type Access = Rc<dyn Fn(&mut Prefs) -> &mut ShortcutState>;
type Change = Rc<dyn Fn(&mut Prefs, Option<&Shortcut>) -> Result<(), ShortcutError>>;

/// Proposes `next` through `change` and records the outcome on the state `access` reaches.
fn propose(s: &mut Prefs, access: &Access, change: &Change, next: Option<Shortcut>) {
    let state = access(s);
    if state.shortcut == next && !state.unregistered {
        state.finish(next, Ok(()));
        return;
    }
    let result = change(s, next.as_ref());
    access(s).finish(next, result);
}

/// A row's shortcut: a keycap that records a chord when pressed (Escape cancels, Backspace
/// clears), and a button that clears a recorded one. `change` registers the proposed chord
/// before releasing the old one; an error keeps the old one and shows the refused chord.
pub(super) struct ShortcutKeycap<'a> {
    key: String,
    label: String,
    state: &'a ShortcutState,
    locked: bool,
    /// Words a conflict instead of the stock refusal, naming what holds the chord.
    conflict: Option<String>,
    require_modifier: bool,
}

impl<'a> ShortcutKeycap<'a> {
    /// `label` names what the shortcut does ("Show Dupin"); the keys are its value.
    pub(super) fn new(key: impl Into<String>, label: impl Into<String>, state: &'a ShortcutState) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            state,
            locked: false,
            conflict: None,
            require_modifier: true,
        }
    }

    pub(super) fn locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }

    pub(super) fn conflict(mut self, conflict: Option<String>) -> Self {
        self.conflict = conflict;
        self
    }

    /// Allows single keys such as Tab or J as well as modifier chords.
    pub(super) fn allow_unmodified(mut self) -> Self {
        self.require_modifier = false;
        self
    }

    pub(super) fn build(
        self,
        k: &Kit,
        access: impl Fn(&mut Prefs) -> &mut ShortcutState + 'static,
        change: impl Fn(&mut Prefs, Option<&Shortcut>) -> Result<(), ShortcutError> + 'static,
    ) -> El {
        let c = &k.c;
        let access: Access = Rc::new(access);
        let change: Change = Rc::new(change);
        let state = self.state;
        let recording = state.is_recording();
        let caps = state.keycaps();
        let keys = caps.join("");
        let (words, ink) = match (caps.is_empty(), recording) {
            (false, _) => (keys.clone(), c.text_base),
            (true, true) => ("Press keys".to_owned(), c.text_muted),
            (true, false) => ("Record".to_owned(), c.text_muted),
        };
        let mut keycap = row()
            .key(&self.key)
            .role("button")
            .label(&self.label)
            .a11y_value(if keys.is_empty() {
                "Not set".to_owned()
            } else {
                keys
            })
            .focusable(!self.locked)
            .disabled(self.locked)
            .focus_ring(2.0, 0.0, c.border_focus)
            .h(24.0)
            .min_w(24.0)
            .px(8.0)
            .center()
            .radius(k.radius.sm)
            .child(text(words).font(k.ty.ui_medium.lh(16.0)).color(ink).a11y_hidden());
        keycap = if let Some(_error) = state.error() {
            keycap.fill(c.bg_base).border(1.0, c.danger.border)
        } else if recording {
            keycap.fill(c.bg_base).border(1.0, c.border_focus)
        } else {
            let mut raised = keycap
                .fill(c.bg_button_neutral)
                .hover(c.surface_raised_hover)
                .press(c.pressed);
            for shadow in &k.elevation.button_neutral {
                raised = raised.box_shadow(*shadow);
            }
            raised
        };
        if recording {
            keycap = keycap.description("Press a shortcut. Escape cancels; Backspace clears.");
        }
        if !self.locked {
            let begin = access.clone();
            keycap = keycap.on_press(move |s: &mut Prefs| begin(s).begin());
        }
        let (input, proposed, locked, require_modifier) =
            (access.clone(), change.clone(), self.locked, self.require_modifier);
        keycap = keycap.on_physical_input(move |s: &mut Prefs, event, _| {
            match input(s).input(event, locked, require_modifier) {
                Step::Pass => false,
                Step::Consumed => true,
                Step::Propose(next) => {
                    propose(s, &input, &proposed, next);
                    true
                }
            }
        });

        // The clear button leads, so the keycap keeps the row's trailing edge set or unset.
        let mut controls = row().gap(4.0).center_y();
        if state.shortcut().is_some() && !recording && !self.locked {
            let (access, change) = (access.clone(), change.clone());
            controls = controls.child(
                row()
                    .key(format!("{}-clear", self.key))
                    .role("button")
                    .label("Clear")
                    .focusable(true)
                    .focus_ring(2.0, 0.0, c.border_focus)
                    .size(24.0, 24.0)
                    .center()
                    .radius(k.radius.sm)
                    .hover(c.hover)
                    .press(c.pressed)
                    .on_press(move |s: &mut Prefs| {
                        access(s).begin();
                        propose(s, &access, &change, None);
                    })
                    .child(icon(&CLOSE, 16.0).color(c.icon_muted)),
            );
        }
        let controls = controls.child(keycap);
        let mut root = col()
            .key(format!("{}-field", self.key))
            .gap(4.0)
            .items_end()
            .child(controls);
        if let Some(error) = state.error() {
            let message = self
                .conflict
                .filter(|_| *error == ShortcutError::Conflict)
                .unwrap_or_else(|| shortcut_error_message(error, state.refused()));
            root = root.child(
                text(message)
                    .key("error")
                    .role("alert")
                    .font(k.ty.small_medium)
                    .color(c.danger.fg)
                    .wrap()
                    .max_w(ERROR_MAX_W),
            );
        }
        // Disabling stops the input but draws the keycap as it was.
        if self.locked {
            root.layer_opacity(0.5)
        } else {
            root
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(state: &mut ShortcutState, key: PhysicalKeyCode, mods: Mods, pressed: bool) -> Step {
        state.input(
            &PhysicalInput::Key {
                key,
                mods,
                pressed,
                repeat: false,
                text: None,
                consumed_mods: Default::default(),
            },
            false,
            true,
        )
    }

    #[test]
    fn a_capture_proposes_a_chord_refuses_one_without_a_modifier_and_swallows_its_release() {
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        let mut state = ShortcutState::new(Some("ctrl+k".parse().unwrap()));
        assert!(matches!(
            key(&mut state, PhysicalKeyCode::KeyJ, ctrl, true),
            Step::Pass
        ));
        state.begin();
        assert!(matches!(
            key(&mut state, PhysicalKeyCode::KeyJ, Mods::default(), true),
            Step::Consumed
        ));
        assert!(state.error().is_some());
        assert_eq!(state.shortcut().unwrap().to_string(), "ctrl+k");
        let Step::Propose(Some(chord)) = key(&mut state, PhysicalKeyCode::KeyQ, ctrl, true) else {
            panic!("⌃Q is proposed");
        };
        state.finish(Some(chord), Ok(()));
        assert!(!state.is_recording());
        assert_eq!(state.keycaps(), ["⌃", "Q"]);
        assert!(matches!(
            key(&mut state, PhysicalKeyCode::KeyQ, ctrl, false),
            Step::Consumed
        ));
        assert!(matches!(
            key(&mut state, PhysicalKeyCode::KeyQ, ctrl, false),
            Step::Pass
        ));
    }

    #[test]
    fn modifiers_show_in_apple_order_and_escape_keeps_the_shortcut() {
        let mut state = ShortcutState::new(Some("cmd+space".parse().unwrap()));
        state.begin();
        let all = Mods {
            ctrl: true,
            alt: true,
            shift: true,
            logo: true,
            ..Mods::default()
        };
        state.input(&PhysicalInput::Modifiers(all), false, true);
        assert_eq!(state.keycaps(), ["⌃", "⌥", "⇧", "⌘"]);
        key(&mut state, PhysicalKeyCode::Escape, Mods::default(), true);
        assert!(!state.is_recording());
        assert_eq!(state.shortcut(), Some(&"cmd+space".parse().unwrap()));
    }
}
