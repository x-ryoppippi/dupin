//! The global shortcut that toggles the panel. A failed registration is never silent: it is
//! logged, posted as a notification when the app can post them, and shown in the panel,
//! which opens to show it, since an accessory app has no other surface. On a first run the
//! welcome in Settings shows it instead.
//!
//! What can't be detected: another app holding the chord through a non-exclusive Carbon
//! registration (Raycast's, for one). Both registrations succeed and macOS delivers the key
//! to one of them, so the README asks users to unassign ⌘Space elsewhere first.

use std::sync::Mutex;

use dupin_mac::system_hotkey;
use rune::{AppHandle, Shortcut, ShortcutError, ShortcutId};

use crate::ui::Ui;

/// The panel's hotkey registration, shared by the panel (which registers it at launch) and
/// the settings window (which replaces it).
pub struct Binding {
    panel: AppHandle<Ui>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    registration: Option<Registration>,
    launch_failure: Option<LaunchFailure>,
}

struct LaunchFailure {
    shortcut: Option<Shortcut>,
    problem: LaunchProblem,
}

#[derive(Clone)]
pub enum LaunchProblem {
    Occupied(String),
    Registration(ShortcutError),
}

struct Registration {
    id: ShortcutId,
    shortcut: Shortcut,
}

impl Binding {
    pub fn new(panel: AppHandle<Ui>) -> Binding {
        Binding {
            panel,
            state: Mutex::default(),
        }
    }

    pub fn launch_problem(&self, shortcut: Option<&Shortcut>) -> Option<LaunchProblem> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let failure = state.launch_failure.as_ref()?;
        (failure.shortcut.as_ref() == shortcut).then(|| failure.problem.clone())
    }

    pub fn refresh_launch_conflict(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let installed = state.shortcut().cloned();
        refresh_system_conflict(
            &mut state.launch_failure,
            installed.as_ref(),
            system_hotkey::owner,
        )
    }

    /// Register the configured hotkey at launch, reporting any failure. Without `announce`,
    /// a failure is only logged: the first-run welcome shows the hotkey and explains the clash
    /// itself, and the panel opening over it would bury that.
    pub fn register_at_launch(&self, accelerator: &str, announce: bool) {
        let shortcut = match parse(accelerator) {
            Ok(Some(shortcut)) => shortcut,
            Ok(None) => return,
            Err(e) => return self.report(accelerator, None, LaunchProblem::Registration(e), announce),
        };
        // Carbon only refuses a chord another app grabbed exclusively. macOS's own shortcuts
        // (Spotlight's ⌘Space) take the key first without making registration fail, so ask who
        // holds it beforehand. Register anyway: freeing the chord later then works without a
        // restart.
        let owner = self.panel.shortcut_owner(&shortcut);
        match self
            .panel
            .register_shortcut(shortcut.clone(), |s: &mut Ui, tx| s.toggle(tx))
        {
            Ok(id) => {
                self.state.lock().unwrap_or_else(|e| e.into_inner()).registration = Some(Registration {
                    id,
                    shortcut: shortcut.clone(),
                });
                crate::trace::registered(accelerator);
                if let Some(owner) = owner {
                    self.report(
                        accelerator,
                        Some(shortcut),
                        LaunchProblem::Occupied(owner),
                        announce,
                    );
                }
            }
            Err(e) => {
                let problem = if e == ShortcutError::Conflict {
                    owner
                        .or_else(|| self.panel.shortcut_owner(&shortcut))
                        .map_or(LaunchProblem::Registration(e), LaunchProblem::Occupied)
                } else {
                    LaunchProblem::Registration(e)
                };
                self.report(accelerator, Some(shortcut), problem, announce);
            }
        }
    }

    /// Swap the hotkey for `next` (none: no hotkey), as the settings window records it. A
    /// chord macOS or another app holds is refused and the old binding stays.
    pub fn replace(&self, next: Option<&Shortcut>) -> Result<(), ShortcutError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(next) = next
            && self.external_owner(&state, next).is_some()
        {
            return Err(ShortcutError::Conflict);
        }
        if state.shortcut() == next {
            state.launch_failure = None;
            return Ok(());
        }
        let new = next
            .map(|shortcut| {
                self.panel
                    .register_shortcut(shortcut.clone(), |s: &mut Ui, tx| s.toggle(tx))
            })
            .transpose()?;
        if let Some(old) = &state.registration
            && let Err(error) = self.panel.unregister_shortcut(old.id)
        {
            if let Some(new) = new {
                let _ = self.panel.unregister_shortcut(new);
            }
            return Err(error);
        }
        state.registration = new.zip(next).map(|(id, shortcut)| Registration {
            id,
            shortcut: shortcut.clone(),
        });
        state.launch_failure = None;
        Ok(())
    }

    pub fn is_installed(&self, shortcut: Option<&Shortcut>) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).shortcut() == shortcut
    }

    /// Who holds `shortcut`, for a refusal message.
    pub fn owner(&self, shortcut: &Shortcut) -> Option<String> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.external_owner(&state, shortcut)
    }

    fn external_owner(&self, state: &State, shortcut: &Shortcut) -> Option<String> {
        if state.shortcut() == Some(shortcut) {
            system_hotkey::owner(shortcut).map(str::to_owned)
        } else {
            self.panel.shortcut_owner(shortcut)
        }
    }

    fn report(&self, accelerator: &str, shortcut: Option<Shortcut>, problem: LaunchProblem, announce: bool) {
        let reason = match &problem {
            LaunchProblem::Occupied(owner) => format!("{owner} already uses it"),
            LaunchProblem::Registration(ShortcutError::Conflict) => "another app already uses it".into(),
            LaunchProblem::Registration(error) => error.to_string(),
        };
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .launch_failure = Some(LaunchFailure { shortcut, problem });
        let message = format!(
            "Couldn't register the {} hotkey: {reason}. Free it in the app that holds it, or pick another in Settings (⌘,).",
            keycaps(accelerator)
        );
        crate::log::warn(&message);
        if !announce {
            return;
        }
        notify(&message);
        self.panel.update(move |s, tx| s.show_notice(message, tx));
    }
}

impl State {
    fn shortcut(&self) -> Option<&Shortcut> {
        self.registration
            .as_ref()
            .map(|registration| &registration.shortcut)
    }
}

fn refresh_system_conflict(
    failure: &mut Option<LaunchFailure>,
    installed: Option<&Shortcut>,
    owner_at: impl FnOnce(&Shortcut) -> Option<&'static str>,
) -> bool {
    let Some(LaunchFailure {
        shortcut: Some(shortcut),
        problem: LaunchProblem::Occupied(owner),
    }) = failure
    else {
        return false;
    };
    if Some(&*shortcut) != installed {
        return false;
    }
    if let Some(current_owner) = owner_at(shortcut) {
        *owner = current_owner.to_owned();
        false
    } else {
        *failure = None;
        true
    }
}

pub fn parse(accelerator: &str) -> Result<Option<Shortcut>, ShortcutError> {
    if accelerator.trim().is_empty() {
        Ok(None)
    } else {
        accelerator.parse().map(Some)
    }
}

fn notify(message: &str) {
    if rune_notify::available() {
        rune_notify::post(
            &rune_notify::Notification::new("hotkey", "Dupin hotkey unavailable").body(message),
        );
    }
}

/// `cmd+space` as macOS writes it: ⌘Space.
pub fn keycaps(accelerator: &str) -> String {
    let mut out = String::new();
    for part in accelerator.split('+').map(str::trim) {
        out.push_str(match part.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "super" | "mod" => "⌘",
            "alt" | "option" | "opt" => "⌥",
            "ctrl" | "control" => "⌃",
            "shift" => "⇧",
            "space" => "Space",
            _ => {
                out.push_str(&part.to_uppercase());
                continue;
            }
        });
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn releasing_a_system_reservation_clears_the_warning_for_an_installed_chord() {
        use super::{LaunchFailure, LaunchProblem, refresh_system_conflict};

        let shortcut: rune::Shortcut = "cmd+space".parse().unwrap();
        let mut failure = Some(LaunchFailure {
            shortcut: Some(shortcut.clone()),
            problem: LaunchProblem::Occupied("Spotlight".into()),
        });

        assert!(!refresh_system_conflict(&mut failure, None, |_| panic!(
            "no registration"
        )));
        assert!(!refresh_system_conflict(
            &mut failure,
            Some(&shortcut),
            |installed| {
                assert_eq!(installed, &shortcut);
                Some("macOS")
            }
        ));
        assert!(
            matches!(&failure, Some(LaunchFailure { problem: LaunchProblem::Occupied(owner), .. }) if owner == "macOS")
        );
        assert!(refresh_system_conflict(&mut failure, Some(&shortcut), |_| None));
        assert!(failure.is_none());
    }

    #[test]
    fn an_empty_hotkey_disables_registration() {
        assert_eq!(super::parse(""), Ok(None));
        assert_eq!(super::parse("  "), Ok(None));
        assert!(super::parse("cmd+not-a-key").is_err());
        assert!(super::parse("cmd+space").unwrap().is_some());
    }

    #[test]
    fn equivalent_spellings_describe_the_same_hotkey() {
        assert_eq!(super::parse("cmd+space"), super::parse("Command+Space"));
    }

    #[test]
    fn a_recorded_shortcut_is_saved_in_a_form_that_reads_back() {
        for accelerator in ["cmd+space", "ctrl+space", "ctrl+alt+k", "shift+cmd+p"] {
            let shortcut: rune::Shortcut = accelerator.parse().unwrap();
            let saved = shortcut.to_string();
            assert_eq!(
                saved.parse::<rune::Shortcut>().unwrap(),
                shortcut,
                "{accelerator} saved as {saved}"
            );
        }
    }

    #[test]
    fn writes_modifiers_as_macos_glyphs() {
        assert_eq!(super::keycaps("cmd+space"), "⌘Space");
        assert_eq!(super::keycaps("ctrl+alt+k"), "⌃⌥K");
    }
}
