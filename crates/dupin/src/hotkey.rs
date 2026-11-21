//! The global shortcut that toggles the panel. A failed registration is never silent: it is
//! logged, posted as a notification when the app can post them, and shown in the panel,
//! which opens to show it, since an accessory app has no other surface.
//!
//! What can't be detected: another app holding the chord through a non-exclusive Carbon
//! registration (Raycast's, for one). Both registrations succeed and macOS delivers the key
//! to one of them, so the README asks users to unassign ⌘Space elsewhere first.

use std::sync::Mutex;

use rune::{AppHandle, Shortcut, ShortcutError, ShortcutId};

use crate::ui::Ui;

/// The panel's hotkey registration, shared by the panel (which registers it at launch) and
/// the settings window (which replaces it).
pub struct Binding {
    panel: AppHandle<Ui>,
    id: Mutex<Option<ShortcutId>>,
    /// Whether the launch registration failed, so recording the same chord again retries it.
    failed: Mutex<bool>,
}

impl Binding {
    pub fn new(panel: AppHandle<Ui>) -> Binding {
        Binding {
            panel,
            id: Mutex::new(None),
            failed: Mutex::new(false),
        }
    }

    pub fn failed_at_launch(&self) -> bool {
        *self.failed.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Register the configured hotkey at launch, reporting any failure.
    pub fn register_at_launch(&self, accelerator: &str) {
        let shortcut = match accelerator.parse::<Shortcut>() {
            Ok(shortcut) => shortcut,
            Err(e) => return self.report(accelerator, &e.to_string()),
        };
        // Carbon only refuses a chord another app grabbed exclusively. macOS's own shortcuts
        // (Spotlight's ⌘Space) take the key first without making registration fail, so ask who
        // holds it beforehand. Register anyway: freeing the chord later then works without a
        // restart.
        if let Some(owner) = self.panel.shortcut_owner(&shortcut) {
            self.report(accelerator, &format!("{owner} already uses it"));
        }
        match self
            .panel
            .register_shortcut(shortcut.clone(), |s: &mut Ui, tx| s.toggle(tx))
        {
            Ok(id) => {
                *self.id.lock().unwrap_or_else(|e| e.into_inner()) = Some(id);
                crate::trace::registered(accelerator);
            }
            Err(e) => self.report(accelerator, &self.why(&shortcut, &e)),
        }
    }

    /// Swap the hotkey for `next` (none: no hotkey), as the settings window records it. A
    /// chord macOS or another app holds is refused and the old binding stays.
    pub fn replace(&self, next: Option<&Shortcut>) -> Result<(), ShortcutError> {
        if let Some(next) = next
            && self
                .panel
                .shortcut_owner(next)
                .is_some_and(|owner| owner != "Dupin")
        {
            return Err(ShortcutError::Conflict);
        }
        let new = next
            .map(|shortcut| {
                self.panel
                    .register_shortcut(shortcut.clone(), |s: &mut Ui, tx| s.toggle(tx))
            })
            .transpose()?;
        let mut id = self.id.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(old) = *id
            && let Err(error) = self.panel.unregister_shortcut(old)
        {
            if let Some(new) = new {
                let _ = self.panel.unregister_shortcut(new);
            }
            return Err(error);
        }
        *id = new;
        *self.failed.lock().unwrap_or_else(|e| e.into_inner()) = false;
        Ok(())
    }

    /// Who holds `shortcut`, for a refusal message.
    pub fn owner(&self, shortcut: &Shortcut) -> Option<String> {
        self.panel.shortcut_owner(shortcut)
    }

    fn report(&self, accelerator: &str, reason: &str) {
        *self.failed.lock().unwrap_or_else(|e| e.into_inner()) = true;
        let message = format!(
            "Couldn't register the {} hotkey: {reason}. Free it in the app that holds it, or pick another in Settings (⌘,).",
            keycaps(accelerator)
        );
        crate::log::warn(&message);
        notify(&message);
        self.panel.update(move |s, tx| s.show_notice(message, tx));
    }

    fn why(&self, shortcut: &Shortcut, error: &ShortcutError) -> String {
        match (error, self.panel.shortcut_owner(shortcut)) {
            (ShortcutError::Conflict, Some(owner)) => format!("{owner} already uses it"),
            (ShortcutError::Conflict, None) => "another app already uses it".to_owned(),
            (e, _) => e.to_string(),
        }
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
